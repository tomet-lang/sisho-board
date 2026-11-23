//! Extension discovery and protocol querying (`--sisho-extension-info` and `--sisho-board-pane`).
//!
//! Security boundary: [`scan`] and [`scan_in`] only inspect directory entries
//! and file permissions on `PATH`; they never execute any discovered program.
//! [`query_info`] and [`query_board_pane`] must only be called for extensions
//! whose [`TrustState`] is [`TrustState::Allowed`].

use std::collections::HashMap;
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use sisho_board_sdk::{BOARD_PANE_FLAG, BoardPane};
use sisho_sdk::{EXTENSION_INFO_FLAG, ExtensionInfo};

use crate::model::ExtensionItem;
use crate::runner::run_with_timeout_env;
use crate::trust::{TrustState, fetch_extensions};

/// True if `word` is a valid extension name (`[a-z0-9-]+`, not starting with `-`).
pub fn is_valid_word(word: &str) -> bool {
    !word.is_empty()
        && !word.starts_with('-')
        && word
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn is_builtin_or_self(word: &str) -> bool {
    matches!(
        word,
        "check" | "fix" | "convert" | "serve" | "spec" | "tomet" | "twrit" | "catasto" | "tandr" | "shelf"
    )
}

/// Discover extensions by delegating to `<sisho_program> --list --json`, falling
/// back safely to [`scan`] with all extensions marked [`TrustState::Untrusted`]
/// if `sisho` is missing or older.
pub fn discover(sisho_program: &Path) -> Vec<ExtensionItem> {
    let Some(entries) = fetch_extensions(sisho_program) else {
        return scan();
    };

    entries
        .into_iter()
        .filter(|entry| entry.word != "board")
        .map(|entry| {
            let (is_active, note) = match entry.status.as_str() {
                "active" => (true, None),
                "shadowed_by_builtin" => (false, Some("shadowed by builtin".to_string())),
                "shadowed_by_duplicate" => (
                    false,
                    entry
                        .shadowed_by
                        .map(|p| format!("shadowed by {p}"))
                        .or_else(|| Some("shadowed by duplicate".to_string())),
                ),
                _ => (false, entry.shadowed_by),
            };
            ExtensionItem {
                word: entry.word,
                path: entry.path,
                canonical_path: entry.canonical_path,
                is_active,
                note,
                trust: entry.trust,
                info: None,
                board_pane: None,
                is_querying: false,
            }
        })
        .collect()
}

/// Scan `PATH` for `sisho-<word>` extension executables (excluding `sisho-board` itself).
///
/// Never executes any discovered binary. All returned items start with
/// [`TrustState::Untrusted`].
pub fn scan() -> Vec<ExtensionItem> {
    let path_var = env::var_os("PATH").unwrap_or_default();
    scan_in(env::split_paths(&path_var))
}

/// Scan specific directories for `sisho-<word>` extension executables.
pub fn scan_in(dirs: impl IntoIterator<Item = PathBuf>) -> Vec<ExtensionItem> {
    let mut raw = Vec::new();
    let mut first_seen: HashMap<String, PathBuf> = HashMap::new();
    let mut order = 0usize;

    for dir in dirs {
        let Ok(read_dir) = fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<_> = read_dir.flatten().collect();
        entries.sort_by_key(|e| e.file_name());

        for entry in entries {
            let file_name = entry.file_name();
            let Some(name_str) = file_name.to_str() else {
                continue;
            };
            let Some(word) = name_str.strip_prefix("sisho-") else {
                continue;
            };
            if !is_valid_word(word) || word == "board" {
                continue;
            }
            let path = entry.path();
            if !is_executable(&path) {
                continue;
            }

            order += 1;
            let is_builtin = is_builtin_or_self(word);
            let (is_active, note) = if is_builtin {
                first_seen.entry(word.to_string()).or_insert_with(|| path.clone());
                (false, Some("shadowed by builtin".to_string()))
            } else if first_seen.contains_key(word) {
                // Skip duplicate PATH entries in the default board view
                continue;
            } else {
                first_seen.insert(word.to_string(), path.clone());
                (true, None)
            };

            let canonical_path = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());

            raw.push((
                order,
                ExtensionItem {
                    word: word.to_string(),
                    path,
                    canonical_path,
                    is_active,
                    note,
                    trust: TrustState::Untrusted,
                    info: None,
                    board_pane: None,
                    is_querying: false,
                },
            ));
        }
    }

    raw.sort_by(|(order_a, a), (order_b, b)| {
        a.word.cmp(&b.word).then_with(|| order_a.cmp(order_b))
    });
    raw.into_iter().map(|(_, item)| item).collect()
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    metadata.is_file() && (metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    metadata.is_file()
}

/// Compute `SISHO_CONFIG_DIR` and `SISHO_DATA_DIR` environment variable pairs for `word`.
///
/// Resolves `$XDG_CONFIG_HOME/sisho/<word>` (fallback `$HOME/.config/sisho/<word>`)
/// and `$XDG_DATA_HOME/sisho/<word>` (fallback `$HOME/.local/share/sisho/<word>`),
/// ignoring `sisho-board`'s own `SISHO_CONFIG_DIR` and `SISHO_DATA_DIR` so board's
/// directories never leak into queried extensions.
pub fn extension_dir_envs(word: &str) -> Vec<(&'static str, PathBuf)> {
    extension_dir_envs_with(word, |key| env::var_os(key))
}

/// Compute `SISHO_CONFIG_DIR` and `SISHO_DATA_DIR` for `word` using a custom env lookup.
pub fn extension_dir_envs_with<F>(word: &str, lookup: F) -> Vec<(&'static str, PathBuf)>
where
    F: Fn(&str) -> Option<OsString>,
{
    let mut envs = Vec::with_capacity(2);
    if let Some(xdg) = lookup("XDG_CONFIG_HOME").filter(|s| !s.to_string_lossy().trim().is_empty())
    {
        envs.push(("SISHO_CONFIG_DIR", PathBuf::from(xdg).join("sisho").join(word)));
    } else if let Some(home) = lookup("HOME").filter(|s| !s.to_string_lossy().trim().is_empty()) {
        envs.push((
            "SISHO_CONFIG_DIR",
            PathBuf::from(home).join(".config").join("sisho").join(word),
        ));
    }

    if let Some(xdg) = lookup("XDG_DATA_HOME").filter(|s| !s.to_string_lossy().trim().is_empty()) {
        envs.push(("SISHO_DATA_DIR", PathBuf::from(xdg).join("sisho").join(word)));
    } else if let Some(home) = lookup("HOME").filter(|s| !s.to_string_lossy().trim().is_empty()) {
        envs.push((
            "SISHO_DATA_DIR",
            PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("sisho")
                .join(word),
        ));
    }
    envs
}

/// Query an allowed extension executable for its `--sisho-extension-info` metadata.
pub fn query_info(word: &str, path: &Path) -> Option<ExtensionInfo> {
    let envs = extension_dir_envs(word);
    let out = run_with_timeout_env(
        path,
        [EXTENSION_INFO_FLAG],
        envs,
        Duration::from_millis(500),
    )?;
    ExtensionInfo::from_json_str(&out).ok()
}

/// Query an allowed extension executable for its `--sisho-board-pane <dir>` data.
pub fn query_board_pane(word: &str, path: &Path, dir: &Path) -> Option<BoardPane> {
    let envs = extension_dir_envs(word);
    let out = run_with_timeout_env(
        path,
        [OsStr::new(BOARD_PANE_FLAG), dir.as_os_str()],
        envs,
        Duration::from_millis(2000),
    )?;
    BoardPane::from_json_str(&out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_dir_envs_ignores_parent_sisho_dirs_and_uses_xdg_or_home() {
        let envs_xdg = extension_dir_envs_with("audit", |key| match key {
            "SISHO_CONFIG_DIR" => Some(OsString::from("/parent/board/config")),
            "SISHO_DATA_DIR" => Some(OsString::from("/parent/board/data")),
            "XDG_CONFIG_HOME" => Some(OsString::from("/xdg/cfg")),
            "XDG_DATA_HOME" => Some(OsString::from("/xdg/data")),
            "HOME" => Some(OsString::from("/home/alice")),
            _ => None,
        });
        assert_eq!(
            envs_xdg,
            vec![
                ("SISHO_CONFIG_DIR", PathBuf::from("/xdg/cfg/sisho/audit")),
                ("SISHO_DATA_DIR", PathBuf::from("/xdg/data/sisho/audit")),
            ]
        );

        let envs_home = extension_dir_envs_with("desk", |key| match key {
            "SISHO_CONFIG_DIR" => Some(OsString::from("/parent/board/config")),
            "SISHO_DATA_DIR" => Some(OsString::from("/parent/board/data")),
            "HOME" => Some(OsString::from("/home/alice")),
            _ => None,
        });
        assert_eq!(
            envs_home,
            vec![
                (
                    "SISHO_CONFIG_DIR",
                    PathBuf::from("/home/alice/.config/sisho/desk")
                ),
                (
                    "SISHO_DATA_DIR",
                    PathBuf::from("/home/alice/.local/share/sisho/desk")
                ),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn discover_uses_sisho_list_json_and_excludes_board() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir =
            std::env::temp_dir().join(format!("sisho-board-discover-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let mock_sisho = temp_dir.join("sisho");
        let script = "#!/bin/sh\n\
            if [ \"$1\" = \"--list\" ] && [ \"$2\" = \"--json\" ]; then\n\
              echo '[{\"word\":\"board\",\"path\":\"/bin/sisho-board\",\"canonical_path\":\"/bin/sisho-board\",\"status\":\"active\",\"shadowed_by\":null,\"trust\":\"allowed\"},{\"word\":\"audit\",\"path\":\"/bin/sisho-audit\",\"canonical_path\":\"/real/sisho-audit\",\"status\":\"active\",\"shadowed_by\":null,\"trust\":\"allowed\"},{\"word\":\"check\",\"path\":\"/bin/sisho-check\",\"canonical_path\":\"/bin/sisho-check\",\"status\":\"shadowed_by_builtin\",\"shadowed_by\":\"builtin\",\"trust\":\"untrusted\"}]'\n\
              exit 0\n\
            fi\n\
            exit 2\n";
        fs::write(&mock_sisho, script).unwrap();
        fs::set_permissions(&mock_sisho, fs::Permissions::from_mode(0o755)).unwrap();

        let items = discover(&mock_sisho);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].word, "audit");
        assert_eq!(items[0].canonical_path, PathBuf::from("/real/sisho-audit"));
        assert!(items[0].is_active);
        assert_eq!(items[0].trust, TrustState::Allowed);

        assert_eq!(items[1].word, "check");
        assert!(!items[1].is_active);
        assert_eq!(items[1].note.as_deref(), Some("shadowed by builtin"));
        assert_eq!(items[1].trust, TrustState::Untrusted);

        let _ = fs::remove_dir_all(&temp_dir);
    }
}

//! Extension discovery and protocol querying (`--sisho-extension-info` and `--sisho-board-pane`).
//!
//! Security boundary: [`scan`] and [`scan_in`] only inspect directory entries
//! and file permissions on `PATH`; they never execute any discovered program.
//! [`query_info`] and [`query_board_pane`] must only be called for extensions
//! whose [`TrustState`] is [`TrustState::Allowed`].

use std::collections::HashMap;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use sisho_board_sdk::{BOARD_PANE_FLAG, BoardPane};
use sisho_sdk::{EXTENSION_INFO_FLAG, ExtensionInfo};

use crate::model::ExtensionItem;
use crate::runner::run_with_timeout;
use crate::trust::TrustState;

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

/// Scan `PATH` for `sisho-<word>` extension executables (excluding `sisho-board` itself).
///
/// Never executes any discovered binary. All returned items start with
/// [`TrustState::Untrusted`]; the caller applies [`crate::trust::TrustConfig`]
/// before deciding whether any extension may be queried.
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

            raw.push((
                order,
                ExtensionItem {
                    word: word.to_string(),
                    path,
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

/// Query an allowed extension executable for its `--sisho-extension-info` metadata.
pub fn query_info(path: &Path) -> Option<ExtensionInfo> {
    let out = run_with_timeout(path, [EXTENSION_INFO_FLAG], Duration::from_millis(500))?;
    ExtensionInfo::from_json_str(&out).ok()
}

/// Query an allowed extension executable for its `--sisho-board-pane <dir>` data.
pub fn query_board_pane(path: &Path, dir: &Path) -> Option<BoardPane> {
    let out = run_with_timeout(
        path,
        [OsStr::new(BOARD_PANE_FLAG), dir.as_os_str()],
        Duration::from_millis(2000),
    )?;
    BoardPane::from_json_str(&out).ok()
}

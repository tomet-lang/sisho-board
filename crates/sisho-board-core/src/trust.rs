//! Extension trust delegation to the `sisho` CLI.
//!
//! `sisho` is the sole owner of `~/.local/share/sisho/trust.json`.
//! `sisho-board` never reads or writes `trust.json` directly; instead it queries
//! `sisho --list --json` for discovered extensions and trust states, and delegates
//! trust updates to `sisho --allow`, `sisho --deny`, or `sisho --untrust`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::runner::{Status, Step, run_with_timeout};

/// Trust permission state for an extension discovered on `PATH`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustState {
    /// Not yet decided by the user (default when absent from `trust.json`).
    /// Never executed automatically.
    #[default]
    Untrusted,
    /// Explicitly allowed by the user. `sisho-board` may query its metadata
    /// and board pane and launch it on request.
    Allowed,
    /// Explicitly denied by the user. Never executed.
    Denied,
}

impl TrustState {
    /// True only when [`TrustState::Allowed`].
    pub fn is_allowed(self) -> bool {
        matches!(self, Self::Allowed)
    }

    /// Lowercase string representation (`"untrusted"`, `"allowed"`, `"denied"`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Untrusted => "untrusted",
            Self::Allowed => "allowed",
            Self::Denied => "denied",
        }
    }
}

/// Extension entry emitted by `sisho --list --json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SishoExtensionEntry {
    /// Extension word (e.g. `"desk"` for `sisho-desk`).
    pub word: String,
    /// Path to the executable on `PATH`.
    pub path: PathBuf,
    /// Canonicalized binary path used by `sisho` to key trust entries.
    pub canonical_path: PathBuf,
    /// Discovery status (`"active"`, `"shadowed_by_builtin"`, `"shadowed_by_duplicate"`).
    pub status: String,
    /// Name or path of the shadowing builtin/binary, if shadowed.
    #[serde(default)]
    pub shadowed_by: Option<String>,
    /// Current trust state reported by `sisho`.
    #[serde(default)]
    pub trust: TrustState,
}

/// Resolve the `sisho` CLI executable (`SISHO_BIN` if set and non-empty, otherwise `"sisho"`).
pub fn default_sisho_program() -> PathBuf {
    resolve_sisho_program_with(|key| std::env::var_os(key))
}

/// Resolve the `sisho` CLI executable using a custom environment lookup function.
pub fn resolve_sisho_program_with<F>(lookup: F) -> PathBuf
where
    F: Fn(&str) -> Option<OsString>,
{
    if let Some(bin) = lookup("SISHO_BIN").filter(|s| !s.to_string_lossy().trim().is_empty()) {
        return PathBuf::from(bin);
    }
    PathBuf::from("sisho")
}

/// Query `<sisho_program> --list --json` and parse the returned JSON array.
///
/// Returns `None` if `sisho` is missing, exits non-zero (e.g. an older `sisho`
/// that does not support `--list --json`), times out, or emits invalid JSON.
pub fn fetch_extensions(sisho_program: &Path) -> Option<Vec<SishoExtensionEntry>> {
    let out = run_with_timeout(
        sisho_program,
        ["--list", "--json"],
        Duration::from_millis(2000),
    )?;
    serde_json::from_str(&out).ok()
}

/// Update extension trust via `<sisho_program> --allow|--deny|--untrust <target>`.
///
/// Passing the extension's `canonical_path` as `target` ensures `sisho` updates
/// the exact binary path.
pub fn set_extension_trust(
    sisho_program: &Path,
    target: &Path,
    new_state: TrustState,
) -> Result<(), String> {
    let flag = match new_state {
        TrustState::Allowed => "--allow",
        TrustState::Denied => "--deny",
        TrustState::Untrusted => "--untrust",
    };

    let outcome = Step::new("sisho trust", sisho_program)
        .arg(flag)
        .arg(target)
        .run();

    if outcome.is_pass() {
        Ok(())
    } else {
        let stderr = outcome.stderr.trim();
        if !stderr.is_empty() {
            Err(stderr.to_string())
        } else {
            Err(match outcome.status {
                Status::Failed { code } => format!("sisho exited with status {code}"),
                Status::Signalled { signal } => format!("sisho terminated by signal {signal}"),
                Status::NotFound => {
                    format!("sisho executable '{}' not found", sisho_program.display())
                }
                Status::CouldNotStart { reason } => reason,
                Status::Passed => unreachable!(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn resolve_sisho_program_uses_sisho_bin_env_when_non_empty() {
        let from_env = resolve_sisho_program_with(|key| match key {
            "SISHO_BIN" => Some(OsString::from("/custom/bin/sisho")),
            _ => None,
        });
        assert_eq!(from_env, PathBuf::from("/custom/bin/sisho"));

        let empty = resolve_sisho_program_with(|key| match key {
            "SISHO_BIN" => Some(OsString::from("  ")),
            _ => None,
        });
        assert_eq!(empty, PathBuf::from("sisho"));

        let unset = resolve_sisho_program_with(|_| None);
        assert_eq!(unset, PathBuf::from("sisho"));
    }

    #[cfg(unix)]
    #[test]
    fn fetch_extensions_and_set_extension_trust_delegate_to_sisho_binary() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir =
            std::env::temp_dir().join(format!("sisho-board-trust-cli-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let log_file = temp_dir.join("calls.log");
        let mock_sisho = temp_dir.join("sisho");
        let script = format!(
            "#!/bin/sh\n\
             echo \"$@\" >> \"{}\"\n\
             if [ \"$1\" = \"--list\" ] && [ \"$2\" = \"--json\" ]; then\n\
               echo '[{{\"word\":\"board\",\"path\":\"/bin/sisho-board\",\"canonical_path\":\"/bin/sisho-board\",\"status\":\"active\",\"shadowed_by\":null,\"trust\":\"allowed\"}},{{\"word\":\"desk\",\"path\":\"/bin/sisho-desk\",\"canonical_path\":\"/real/sisho-desk\",\"status\":\"active\",\"shadowed_by\":null,\"trust\":\"untrusted\"}}]'\n\
               exit 0\n\
             fi\n\
             if [ \"$1\" = \"--allow\" ] || [ \"$1\" = \"--deny\" ] || [ \"$1\" = \"--untrust\" ]; then\n\
               exit 0\n\
             fi\n\
             echo 'unsupported flag' >&2\n\
             exit 2\n",
            log_file.display()
        );
        fs::write(&mock_sisho, script).unwrap();
        fs::set_permissions(&mock_sisho, fs::Permissions::from_mode(0o755)).unwrap();

        let entries = fetch_extensions(&mock_sisho).expect("mock sisho --list --json should parse");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].word, "desk");
        assert_eq!(entries[1].canonical_path, PathBuf::from("/real/sisho-desk"));
        assert_eq!(entries[1].trust, TrustState::Untrusted);

        let target = Path::new("/real/sisho-desk");
        set_extension_trust(&mock_sisho, target, TrustState::Allowed).unwrap();
        set_extension_trust(&mock_sisho, target, TrustState::Denied).unwrap();
        set_extension_trust(&mock_sisho, target, TrustState::Untrusted).unwrap();

        let log = fs::read_to_string(&log_file).unwrap();
        let lines: Vec<&str> = log.lines().collect();
        assert_eq!(
            lines,
            vec![
                "--list --json",
                "--allow /real/sisho-desk",
                "--deny /real/sisho-desk",
                "--untrust /real/sisho-desk",
            ]
        );

        // Non-existent binary returns None on fetch and Err on set
        let missing = temp_dir.join("missing-sisho");
        assert!(fetch_extensions(&missing).is_none());
        let err = set_extension_trust(&missing, target, TrustState::Allowed).unwrap_err();
        assert!(err.contains("not found"), "{err}");

        let _ = fs::remove_dir_all(&temp_dir);
    }
}

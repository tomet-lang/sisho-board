//! User trust configuration (`~/.config/sisho/board.json`).
//!
//! By default, discovered extensions on `PATH` are [`TrustState::Untrusted`].
//! `sisho-board` never executes an extension subprocess (`--sisho-extension-info`,
//! `--sisho-board-pane`, editor launch, or terminal handover) unless the user
//! has explicitly marked it [`TrustState::Allowed`].

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Current schema version for `board.json`.
pub const CURRENT_TRUST_SCHEMA_VERSION: u32 = 1;

const fn default_schema_version() -> u32 {
    CURRENT_TRUST_SCHEMA_VERSION
}

/// Trust permission state for an extension discovered on `PATH`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustState {
    /// Not yet decided by the user (default when absent from `board.json`).
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

    /// Cycle trust state when the user toggles permission in the TUI:
    /// - `Untrusted` -> `Allowed`
    /// - `Allowed` -> `Denied`
    /// - `Denied` -> `Allowed`
    pub fn toggle(self) -> Self {
        match self {
            Self::Untrusted => Self::Allowed,
            Self::Allowed => Self::Denied,
            Self::Denied => Self::Allowed,
        }
    }
}

/// Persistent trust configuration stored at `~/.config/sisho/board.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustConfig {
    /// Schema version, currently 1.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// Map of extension word (e.g. `"desk"`) to its [`TrustState`].
    #[serde(default)]
    pub extensions: BTreeMap<String, TrustState>,
}

impl Default for TrustConfig {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_TRUST_SCHEMA_VERSION,
            extensions: BTreeMap::new(),
        }
    }
}

impl TrustConfig {
    /// Create an empty trust configuration where all extensions are `Untrusted`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up the trust state for `word`. Defaults to [`TrustState::Untrusted`] if not present.
    pub fn trust_of(&self, word: &str) -> TrustState {
        self.extensions.get(word).copied().unwrap_or(TrustState::Untrusted)
    }

    /// Set the trust state for `word`.
    pub fn set_trust(&mut self, word: impl Into<String>, state: TrustState) {
        let word = word.into();
        match state {
            TrustState::Untrusted => {
                self.extensions.remove(&word);
            }
            TrustState::Allowed | TrustState::Denied => {
                self.extensions.insert(word, state);
            }
        }
    }

    /// Load `TrustConfig` from `path`. Returns [`TrustConfig::default()`] if the
    /// file does not exist or cannot be parsed.
    pub fn load_from(path: &Path) -> Self {
        let Ok(contents) = fs::read_to_string(path) else {
            return Self::default();
        };
        serde_json::from_str(&contents).unwrap_or_default()
    }

    /// Save `TrustConfig` to `path` as pretty-printed JSON, creating parent
    /// directories if needed.
    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        let mut json = serde_json::to_string_pretty(self)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        json.push('\n');
        fs::write(path, json)
    }
}

/// Resolve the default path to `board.json`:
/// `$XDG_CONFIG_HOME/sisho/board.json` if `XDG_CONFIG_HOME` is set and non-empty,
/// otherwise `$HOME/.config/sisho/board.json`.
pub fn default_config_path() -> Option<PathBuf> {
    resolve_config_path_with(|key| std::env::var(key).ok())
}

/// Resolve the path to `board.json` using a custom environment variable lookup function.
pub fn resolve_config_path_with<F>(lookup: F) -> Option<PathBuf>
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(xdg) = lookup("XDG_CONFIG_HOME").filter(|s| !s.trim().is_empty()) {
        return Some(PathBuf::from(xdg).join("sisho").join("board.json"));
    }
    if let Some(home) = lookup("HOME").filter(|s| !s.trim().is_empty()) {
        return Some(
            PathBuf::from(home)
                .join(".config")
                .join("sisho")
                .join("board.json"),
        );
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_config_path_prefers_xdg_over_home() {
        let path = resolve_config_path_with(|key| match key {
            "XDG_CONFIG_HOME" => Some("/custom/xdg".to_string()),
            "HOME" => Some("/home/alice".to_string()),
            _ => None,
        });
        assert_eq!(path, Some(PathBuf::from("/custom/xdg/sisho/board.json")));

        let fallback = resolve_config_path_with(|key| match key {
            "XDG_CONFIG_HOME" => Some("".to_string()),
            "HOME" => Some("/home/alice".to_string()),
            _ => None,
        });
        assert_eq!(
            fallback,
            Some(PathBuf::from("/home/alice/.config/sisho/board.json"))
        );
    }

    #[test]
    fn trust_state_defaults_to_untrusted_and_toggles_cleanly() {
        let mut cfg = TrustConfig::new();
        assert_eq!(cfg.trust_of("desk"), TrustState::Untrusted);

        let next = cfg.trust_of("desk").toggle();
        assert_eq!(next, TrustState::Allowed);
        cfg.set_trust("desk", next);
        assert_eq!(cfg.trust_of("desk"), TrustState::Allowed);

        let denied = cfg.trust_of("desk").toggle();
        assert_eq!(denied, TrustState::Denied);
        cfg.set_trust("desk", denied);
        assert_eq!(cfg.trust_of("desk"), TrustState::Denied);

        let allowed_again = cfg.trust_of("desk").toggle();
        assert_eq!(allowed_again, TrustState::Allowed);
    }

    #[test]
    fn load_and_save_board_json_roundtrip() {
        let temp_dir =
            std::env::temp_dir().join(format!("sisho-board-trust-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp_dir);
        let config_path = temp_dir.join("sisho").join("board.json");

        // Missing file loads as default (all untrusted)
        let loaded_missing = TrustConfig::load_from(&config_path);
        assert_eq!(loaded_missing, TrustConfig::default());

        let mut cfg = TrustConfig::new();
        cfg.set_trust("desk", TrustState::Allowed);
        cfg.set_trust("suspicious", TrustState::Denied);
        cfg.save_to(&config_path).unwrap();

        let raw = fs::read_to_string(&config_path).unwrap();
        assert!(raw.contains(r#""desk": "allowed""#), "{raw}");
        assert!(raw.contains(r#""suspicious": "denied""#), "{raw}");

        let reloaded = TrustConfig::load_from(&config_path);
        assert_eq!(reloaded, cfg);
        assert_eq!(reloaded.trust_of("desk"), TrustState::Allowed);
        assert_eq!(reloaded.trust_of("suspicious"), TrustState::Denied);
        assert_eq!(reloaded.trust_of("unknown"), TrustState::Untrusted);

        let _ = fs::remove_dir_all(&temp_dir);
    }
}

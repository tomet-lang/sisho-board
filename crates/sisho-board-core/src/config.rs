//! Portable UI board configuration (`$SISHO_CONFIG_DIR/board.json` or `$XDG_CONFIG_HOME/sisho/board/board.json`).
//!
//! `BoardConfig` stores only portable UI preferences (such as hidden extension panes)
//! with zero binary paths and zero trust state. Extension execution trust is owned
//! exclusively by `sisho` (`~/.local/share/sisho/trust.json`).

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Current schema version for `board.json`.
pub const CURRENT_BOARD_CONFIG_SCHEMA_VERSION: u32 = 1;

const fn default_schema_version() -> u32 {
    CURRENT_BOARD_CONFIG_SCHEMA_VERSION
}

/// Portable UI customization for `sisho-board` (`board.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardConfig {
    /// Schema version, currently 1.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// Extension words whose board panes should be hidden without affecting trust.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub hidden_extension_panes: BTreeSet<String>,
}

impl Default for BoardConfig {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_BOARD_CONFIG_SCHEMA_VERSION,
            hidden_extension_panes: BTreeSet::new(),
        }
    }
}

impl BoardConfig {
    /// Create a default `BoardConfig` with no hidden panes.
    pub fn new() -> Self {
        Self::default()
    }

    /// True if `word`'s extension pane is hidden by user UI configuration.
    pub fn is_pane_hidden(&self, word: &str) -> bool {
        self.hidden_extension_panes.contains(word)
    }

    /// Load `BoardConfig` from `path`. Returns [`BoardConfig::default()`] if the
    /// file does not exist or cannot be parsed.
    pub fn load_from(path: &Path) -> Self {
        let Ok(contents) = fs::read_to_string(path) else {
            return Self::default();
        };
        serde_json::from_str(&contents).unwrap_or_default()
    }

    /// Save `BoardConfig` to `path` as pretty-printed JSON, creating parent
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
/// - `$SISHO_CONFIG_DIR/board.json` if `SISHO_CONFIG_DIR` is set and non-empty
/// - otherwise `$XDG_CONFIG_HOME/sisho/board/board.json`
/// - otherwise `$HOME/.config/sisho/board/board.json`
///
/// Matches `sisho_sdk::config_dir("board")`.
pub fn default_config_path() -> Option<PathBuf> {
    resolve_config_path_with(|key| std::env::var(key).ok())
}

/// Resolve the path to `board.json` using a custom environment variable lookup function.
///
/// Matches `sisho_sdk::config_dir("board")`.
pub fn resolve_config_path_with<F>(lookup: F) -> Option<PathBuf>
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(dir) = lookup("SISHO_CONFIG_DIR").filter(|s| !s.trim().is_empty()) {
        return Some(PathBuf::from(dir).join("board.json"));
    }
    if let Some(xdg) = lookup("XDG_CONFIG_HOME").filter(|s| !s.trim().is_empty()) {
        return Some(
            PathBuf::from(xdg)
                .join("sisho")
                .join("board")
                .join("board.json"),
        );
    }
    if let Some(home) = lookup("HOME").filter(|s| !s.trim().is_empty()) {
        return Some(
            PathBuf::from(home)
                .join(".config")
                .join("sisho")
                .join("board")
                .join("board.json"),
        );
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_config_path_precedence_matches_sisho_sdk() {
        let with_sisho_dir = resolve_config_path_with(|key| match key {
            "SISHO_CONFIG_DIR" => Some("/custom/sisho-board".to_string()),
            "XDG_CONFIG_HOME" => Some("/custom/xdg".to_string()),
            "HOME" => Some("/home/alice".to_string()),
            _ => None,
        });
        assert_eq!(
            with_sisho_dir,
            Some(PathBuf::from("/custom/sisho-board/board.json"))
        );

        let with_xdg = resolve_config_path_with(|key| match key {
            "SISHO_CONFIG_DIR" => Some("  ".to_string()),
            "XDG_CONFIG_HOME" => Some("/custom/xdg".to_string()),
            "HOME" => Some("/home/alice".to_string()),
            _ => None,
        });
        assert_eq!(
            with_xdg,
            Some(PathBuf::from("/custom/xdg/sisho/board/board.json"))
        );

        let fallback = resolve_config_path_with(|key| match key {
            "XDG_CONFIG_HOME" => Some("".to_string()),
            "HOME" => Some("/home/alice".to_string()),
            _ => None,
        });
        assert_eq!(
            fallback,
            Some(PathBuf::from("/home/alice/.config/sisho/board/board.json"))
        );
    }

    #[test]
    fn load_and_save_board_config_roundtrip() {
        let temp_dir = std::env::temp_dir().join(format!(
            "sisho-board-config-test-{}",
            crate::unique_test_suffix()
        ));
        let _ = fs::remove_dir_all(&temp_dir);
        let config_path = temp_dir.join("sisho").join("board").join("board.json");

        // Missing file loads as default
        let loaded_missing = BoardConfig::load_from(&config_path);
        assert_eq!(loaded_missing, BoardConfig::default());

        // Empty hidden_extension_panes is omitted on save
        let empty_cfg = BoardConfig::new();
        empty_cfg.save_to(&config_path).unwrap();
        let raw_empty = fs::read_to_string(&config_path).unwrap();
        assert!(!raw_empty.contains("hidden_extension_panes"), "{raw_empty}");
        assert!(raw_empty.contains(r#""schema_version": 1"#), "{raw_empty}");

        // Non-empty hidden_extension_panes roundtrips cleanly
        let mut cfg = BoardConfig::new();
        cfg.hidden_extension_panes.insert("audit".to_string());
        cfg.save_to(&config_path).unwrap();

        let raw = fs::read_to_string(&config_path).unwrap();
        assert!(raw.contains(r#""audit""#), "{raw}");

        let reloaded = BoardConfig::load_from(&config_path);
        assert_eq!(reloaded, cfg);
        assert!(reloaded.is_pane_hidden("audit"));
        assert!(!reloaded.is_pane_hidden("desk"));

        let _ = fs::remove_dir_all(&temp_dir);
    }
}

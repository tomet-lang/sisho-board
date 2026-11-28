//! Layer 0: Portable UI board configuration (`$SISHO_CONFIG_DIR/board.json` or
//! `$XDG_CONFIG_HOME/sisho/board/board.json`).
//!
//! `BoardConfig` stores only portable UI preferences (customizable board views,
//! pane layouts, and hidden extension panes) with zero binary paths and zero
//! trust state. Extension execution trust is owned exclusively by `sisho`
//! (`~/.local/share/sisho/trust.json`).

use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Current schema version for `board.json`.
pub const CURRENT_BOARD_CONFIG_SCHEMA_VERSION: u32 = 2;

/// Default left-column width percentage for a [`BoardView`].
pub const DEFAULT_LEFT_RATIO: u16 = 45;

/// Minimum allowed left-column width percentage.
pub const MIN_LEFT_RATIO: u16 = 20;

/// Maximum allowed left-column width percentage.
pub const MAX_LEFT_RATIO: u16 = 80;

/// Default vertical weight for a newly added [`PaneSlot`].
pub const DEFAULT_PANE_WEIGHT: u16 = 25;

/// Minimum allowed vertical weight for a [`PaneSlot`] when normalized.
pub const MIN_PANE_WEIGHT: u16 = 1;

/// Minimum allowed vertical weight when adjusting in the layout editor.
pub const MIN_EDITOR_PANE_WEIGHT: u16 = 5;

/// Maximum allowed vertical weight for a [`PaneSlot`].
pub const MAX_PANE_WEIGHT: u16 = 100;

const fn default_schema_version() -> u32 {
    CURRENT_BOARD_CONFIG_SCHEMA_VERSION
}

const fn default_left_ratio() -> u16 {
    DEFAULT_LEFT_RATIO
}

const fn default_pane_weight() -> u16 {
    DEFAULT_PANE_WEIGHT
}

/// Identifier of a pane that can be placed in a [`BoardView`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaneId {
    #[default]
    Checks,
    Health,
    Extensions,
    BrokenLinks,
    ExtensionPanes,
    OutputPreview,
    /// Standalone pane backed by a specific extension's `--sisho-board-pane` output.
    Extension(String),
}

impl PaneId {
    /// All built-in pane identifiers in canonical display order.
    pub const BUILTIN_ALL: [Self; 6] = [
        Self::Checks,
        Self::Health,
        Self::Extensions,
        Self::BrokenLinks,
        Self::ExtensionPanes,
        Self::OutputPreview,
    ];

    /// Human-readable display label for this pane.
    pub fn label(&self) -> String {
        match self {
            Self::Checks => "Checks".to_string(),
            Self::Health => "Health".to_string(),
            Self::Extensions => "Tools & Extensions".to_string(),
            Self::BrokenLinks => "Broken Links".to_string(),
            Self::ExtensionPanes => "Extension Panes".to_string(),
            Self::OutputPreview => "Output Preview".to_string(),
            Self::Extension(word) => format!("Ext: {word}"),
        }
    }

    /// True if this pane is one of the built-in dashboard panes.
    pub fn is_builtin(&self) -> bool {
        !matches!(self, Self::Extension(_))
    }

    /// Return the extension word if this is a standalone [`PaneId::Extension`].
    pub fn extension_word(&self) -> Option<&str> {
        match self {
            Self::Extension(word) => Some(word.as_str()),
            _ => None,
        }
    }
}

/// A pane placement and relative vertical height weight within a column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneSlot {
    pub pane: PaneId,
    #[serde(default = "default_pane_weight")]
    pub weight: u16,
}

impl PaneSlot {
    /// Create a new [`PaneSlot`] with `weight` clamped to `1..=100`.
    pub fn new(pane: PaneId, weight: u16) -> Self {
        Self {
            pane,
            weight: weight.clamp(MIN_PANE_WEIGHT, MAX_PANE_WEIGHT),
        }
    }

    fn normalize(&mut self) {
        self.weight = self.weight.clamp(MIN_PANE_WEIGHT, MAX_PANE_WEIGHT);
    }
}

/// A named two-column layout preset in `board.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardView {
    pub name: String,
    #[serde(default = "default_left_ratio")]
    pub left_ratio: u16,
    #[serde(default)]
    pub left: Vec<PaneSlot>,
    #[serde(default)]
    pub right: Vec<PaneSlot>,
}

impl BoardView {
    /// Create the default `"Overview"` board view matching the initial dashboard layout.
    pub fn default_overview() -> Self {
        Self {
            name: "Overview".to_string(),
            left_ratio: DEFAULT_LEFT_RATIO,
            left: vec![
                PaneSlot::new(PaneId::Extensions, 55),
                PaneSlot::new(PaneId::BrokenLinks, 45),
            ],
            right: vec![
                PaneSlot::new(PaneId::Health, 25),
                PaneSlot::new(PaneId::Checks, 25),
                PaneSlot::new(PaneId::ExtensionPanes, 20),
                PaneSlot::new(PaneId::OutputPreview, 30),
            ],
        }
    }

    /// All visible panes in this view (left column top-to-bottom, then right column).
    pub fn visible_panes(&self) -> Vec<&PaneId> {
        self.left
            .iter()
            .chain(self.right.iter())
            .map(|slot| &slot.pane)
            .collect()
    }

    /// True if `pane` appears in either column of this view.
    pub fn contains(&self, pane: &PaneId) -> bool {
        self.left.iter().any(|slot| &slot.pane == pane)
            || self.right.iter().any(|slot| &slot.pane == pane)
    }

    /// Clamp ratios/weights, normalize blank names, and remove duplicate pane entries.
    pub fn normalize(&mut self) {
        if self.name.trim().is_empty() {
            self.name = "View".to_string();
        }
        self.left_ratio = self.left_ratio.clamp(MIN_LEFT_RATIO, MAX_LEFT_RATIO);

        let mut seen = HashSet::new();
        self.left.retain_mut(|slot| {
            slot.normalize();
            seen.insert(slot.pane.clone())
        });
        self.right.retain_mut(|slot| {
            slot.normalize();
            seen.insert(slot.pane.clone())
        });
    }
}

fn default_views() -> Vec<BoardView> {
    vec![BoardView::default_overview()]
}

/// Portable UI customization for `sisho-board` (`board.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardConfig {
    /// Schema version, currently 2.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// Ordered list of named board views; always contains at least one view.
    #[serde(default = "default_views")]
    pub views: Vec<BoardView>,
    /// Index of the currently active view in [`BoardConfig::views`].
    #[serde(default)]
    pub active_view: usize,
    /// Extension words filtered out of the aggregate [`PaneId::ExtensionPanes`] pane
    /// without affecting trust.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub hidden_extension_panes: BTreeSet<String>,
}

impl Default for BoardConfig {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_BOARD_CONFIG_SCHEMA_VERSION,
            views: default_views(),
            active_view: 0,
            hidden_extension_panes: BTreeSet::new(),
        }
    }
}

impl BoardConfig {
    /// Create a default `BoardConfig` with the `"Overview"` view and no hidden panes.
    pub fn new() -> Self {
        Self::default()
    }

    /// Ensure invariants: non-empty `views`, normalized slots/ratios, valid `active_view`,
    /// and current schema version.
    pub fn normalize(&mut self) {
        self.schema_version = CURRENT_BOARD_CONFIG_SCHEMA_VERSION;
        if self.views.is_empty() {
            self.views = default_views();
        }
        for view in &mut self.views {
            view.normalize();
        }
        self.active_view = self.active_view.min(self.views.len() - 1);
    }

    /// Return a reference to the currently active [`BoardView`].
    pub fn active_view(&self) -> &BoardView {
        let idx = self.active_view.min(self.views.len().saturating_sub(1));
        &self.views[idx]
    }

    /// Return a mutable reference to the currently active [`BoardView`].
    pub fn active_view_mut(&mut self) -> &mut BoardView {
        if self.views.is_empty() {
            self.views = default_views();
        }
        let idx = self.active_view.min(self.views.len() - 1);
        self.active_view = idx;
        &mut self.views[idx]
    }

    /// True if `word`'s extension pane is hidden from the aggregate
    /// [`PaneId::ExtensionPanes`] view by user UI configuration.
    pub fn is_pane_hidden(&self, word: &str) -> bool {
        self.hidden_extension_panes.contains(word)
    }

    /// Load `BoardConfig` from `path` and normalize invariants. Returns
    /// [`BoardConfig::default()`] if the file does not exist or cannot be parsed.
    pub fn load_from(path: &Path) -> Self {
        let Ok(contents) = fs::read_to_string(path) else {
            return Self::default();
        };
        let mut config: Self = serde_json::from_str(&contents).unwrap_or_default();
        config.normalize();
        config
    }

    /// Save normalized `BoardConfig` to `path` as pretty-printed JSON, creating
    /// parent directories if needed.
    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        let mut normalized = self.clone();
        normalized.normalize();
        let mut json = serde_json::to_string_pretty(&normalized)
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
pub fn resolve_config_path() -> Option<PathBuf> {
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
fn unique_test_suffix() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{nanos}-{seq}")
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
    fn pane_id_helpers_and_view_queries() {
        assert!(PaneId::Checks.is_builtin());
        assert_eq!(PaneId::Extensions.label(), "Tools & Extensions");
        let ext = PaneId::Extension("history".to_string());
        assert!(!ext.is_builtin());
        assert_eq!(ext.label(), "Ext: history");
        assert_eq!(ext.extension_word(), Some("history"));

        let view = BoardView::default_overview();
        assert_eq!(view.name, "Overview");
        assert_eq!(view.left_ratio, 45);
        assert_eq!(
            view.visible_panes(),
            vec![
                &PaneId::Extensions,
                &PaneId::BrokenLinks,
                &PaneId::Health,
                &PaneId::Checks,
                &PaneId::ExtensionPanes,
                &PaneId::OutputPreview,
            ]
        );
        assert!(view.contains(&PaneId::Checks));
        assert!(!view.contains(&ext));
    }

    #[test]
    fn load_and_save_board_config_roundtrip() {
        let temp_dir = std::env::temp_dir().join(format!(
            "sisho-board-config-test-{}",
            unique_test_suffix()
        ));
        let _ = fs::remove_dir_all(&temp_dir);
        let config_path = temp_dir.join("sisho").join("board").join("board.json");

        // Missing file loads as default
        let loaded_missing = BoardConfig::load_from(&config_path);
        assert_eq!(loaded_missing, BoardConfig::default());

        // Empty hidden_extension_panes is omitted on save; schema_version is 2
        let empty_cfg = BoardConfig::new();
        empty_cfg.save_to(&config_path).unwrap();
        let raw_empty = fs::read_to_string(&config_path).unwrap();
        assert!(!raw_empty.contains("hidden_extension_panes"), "{raw_empty}");
        assert!(raw_empty.contains(r#""schema_version": 2"#), "{raw_empty}");

        // Custom views, standalone extension pane, and hidden_extension_panes roundtrip cleanly
        let mut cfg = BoardConfig::new();
        cfg.hidden_extension_panes.insert("audit".to_string());
        cfg.views.push(BoardView {
            name: "Focus".to_string(),
            left_ratio: 35,
            left: vec![PaneSlot::new(PaneId::Extension("history".to_string()), 60)],
            right: vec![
                PaneSlot::new(PaneId::Checks, 40),
                PaneSlot::new(PaneId::OutputPreview, 60),
            ],
        });
        cfg.active_view = 1;
        cfg.save_to(&config_path).unwrap();

        let raw = fs::read_to_string(&config_path).unwrap();
        assert!(raw.contains(r#""audit""#), "{raw}");
        assert!(raw.contains(r#""history""#), "{raw}");

        let reloaded = BoardConfig::load_from(&config_path);
        assert_eq!(reloaded, cfg);
        assert!(reloaded.is_pane_hidden("audit"));
        assert!(!reloaded.is_pane_hidden("desk"));
        assert_eq!(reloaded.active_view().name, "Focus");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn load_normalizes_empty_views_out_of_bounds_values_and_duplicates() {
        let temp_dir = std::env::temp_dir().join(format!(
            "sisho-board-config-norm-{}",
            unique_test_suffix()
        ));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();
        let config_path = temp_dir.join("board.json");

        // Empty views falls back to default views and clamps active_view
        fs::write(
            &config_path,
            r#"{ "schema_version": 2, "views": [], "active_view": 9 }"#,
        )
        .unwrap();
        let loaded_empty = BoardConfig::load_from(&config_path);
        assert_eq!(loaded_empty.views, vec![BoardView::default_overview()]);
        assert_eq!(loaded_empty.active_view, 0);

        // Clamps left_ratio (20..=80), weight (>= 1), active_view, and deduplicates panes
        fs::write(
            &config_path,
            r#"{
                "schema_version": 2,
                "active_view": 4,
                "views": [
                    {
                        "name": "   ",
                        "left_ratio": 5,
                        "left": [
                            { "pane": "checks", "weight": 0 },
                            { "pane": "checks", "weight": 50 }
                        ],
                        "right": [
                            { "pane": "checks", "weight": 20 },
                            { "pane": "output_preview", "weight": 250 }
                        ]
                    },
                    {
                        "name": "Wide",
                        "left_ratio": 95,
                        "left": [{ "pane": "health", "weight": 10 }],
                        "right": []
                    }
                ]
            }"#,
        )
        .unwrap();

        let loaded = BoardConfig::load_from(&config_path);
        assert_eq!(loaded.active_view, 1);
        assert_eq!(loaded.views[0].name, "View");
        assert_eq!(loaded.views[0].left_ratio, 20);
        assert_eq!(
            loaded.views[0].left,
            vec![PaneSlot::new(PaneId::Checks, 1)]
        );
        assert_eq!(
            loaded.views[0].right,
            vec![PaneSlot::new(PaneId::OutputPreview, 100)]
        );
        assert_eq!(loaded.views[1].left_ratio, 80);

        let _ = fs::remove_dir_all(&temp_dir);
    }
}

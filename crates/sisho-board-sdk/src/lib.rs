//! Layer 0: Extension Board Pane disclosure protocol (`--sisho-board-pane`).
//!
//! An extension that supports board panes can be queried by `sisho-board` with
//! `sisho-<word> --sisho-board-pane [path]`. The extension inspects the vault
//! or repository at `path` (default `.`), prints a single [`BoardPane`] JSON
//! object to standard output, and exits with code 0.

use std::collections::BTreeMap;
use std::io::{self, Write};

use serde::{Deserialize, Serialize};

/// Current schema version for extension board panes.
pub const CURRENT_BOARD_SCHEMA_VERSION: u32 = 1;

/// The standard CLI flag that requests an extension's board pane data.
pub const BOARD_PANE_FLAG: &str = "--sisho-board-pane";

const fn default_schema_version() -> u32 {
    CURRENT_BOARD_SCHEMA_VERSION
}

/// Structured board pane data emitted by an extension when invoked with `--sisho-board-pane`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardPane {
    /// Schema version, currently 1.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// Pane title displayed on the Board (e.g. `"Readiness & Quality"`).
    pub title: String,
    /// Optional overall progress or score gauge displayed at the top of the pane.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gauge: Option<BoardGauge>,
    /// Selectable status items displayed within the pane.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<BoardItem>,
    /// Overall detail or preview text shown in the output preview pane when no
    /// specific item detail overrides it, or as a header.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Additional custom fields for forward compatibility.
    #[serde(flatten, default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl BoardPane {
    /// Create a new `BoardPane` with schema version 1 and the given title.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            schema_version: CURRENT_BOARD_SCHEMA_VERSION,
            title: title.into(),
            gauge: None,
            items: Vec::new(),
            summary: None,
            extra: BTreeMap::new(),
        }
    }

    /// Set the gauge for this pane.
    pub fn with_gauge(mut self, gauge: BoardGauge) -> Self {
        self.gauge = Some(gauge);
        self
    }

    /// Replace all items in this pane.
    pub fn with_items(mut self, items: impl IntoIterator<Item = BoardItem>) -> Self {
        self.items = items.into_iter().collect();
        self
    }

    /// Append a single item to this pane.
    pub fn add_item(mut self, item: BoardItem) -> Self {
        self.items.push(item);
        self
    }

    /// Set the summary text for this pane.
    pub fn with_summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = Some(summary.into());
        self
    }

    /// Serialize to a pretty-printed JSON string.
    pub fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Serialize to a compact JSON string.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Parse `BoardPane` from a JSON string.
    pub fn from_json_str(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Parse `BoardPane` from an arbitrary reader.
    pub fn from_reader<R: io::Read>(reader: R) -> Result<Self, serde_json::Error> {
        serde_json::from_reader(reader)
    }

    /// Write pretty JSON to `writer` followed by a newline and flush.
    pub fn print_to<W: Write>(&self, mut writer: W) -> io::Result<()> {
        serde_json::to_writer_pretty(&mut writer, self)?;
        writer.write_all(b"\n")?;
        writer.flush()
    }

    /// Write pretty JSON to standard output followed by a newline and flush.
    pub fn print(&self) -> io::Result<()> {
        self.print_to(io::stdout().lock())
    }
}

/// A progress or percentage gauge displayed in a [`BoardPane`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardGauge {
    /// Short label for the gauge (e.g. `"Vault Readiness"`).
    pub label: String,
    /// Completion percentage, clamped to `0..=100`.
    pub percent: u8,
}

impl BoardGauge {
    /// Create a new `BoardGauge` with `percent` clamped to `0..=100`.
    pub fn new(label: impl Into<String>, percent: u8) -> Self {
        Self {
            label: label.into(),
            percent: percent.min(100),
        }
    }
}

/// Visual status indicator for a [`BoardItem`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoardStatus {
    /// Passing state (rendered with `✓` in green).
    Pass,
    /// Warning state (rendered with `!` in yellow).
    Warn,
    /// Failing state (rendered with `✗` in red).
    Fail,
    /// Informational state (rendered with `•` in cyan/gray).
    #[default]
    Info,
}

/// A single row item within a [`BoardPane`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardItem {
    /// Item label (e.g. `"Metadata Coverage"`).
    pub label: String,
    /// Item value summary (e.g. `"95% (38/40)"` or `"100%"`).
    pub value: String,
    /// Status badge for this item.
    #[serde(default)]
    pub status: BoardStatus,
    /// Detailed output or explanation shown in the Step Output Preview pane when this item is selected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl BoardItem {
    /// Create a new `BoardItem` with [`BoardStatus::Info`] and no detail.
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            status: BoardStatus::Info,
            detail: None,
        }
    }

    /// Set the status of this item.
    pub fn with_status(mut self, status: BoardStatus) -> Self {
        self.status = status;
        self
    }

    /// Set the detailed preview text for this item.
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn board_pane_roundtrip_serialization() {
        let pane = BoardPane::new("Readiness & Quality")
            .with_gauge(BoardGauge::new("Overall Score", 92))
            .add_item(
                BoardItem::new("Metadata Coverage", "95% (38/40)")
                    .with_status(BoardStatus::Pass)
                    .with_detail("Missing id in:\n- docs/draft1.tmt\n- docs/draft2.tmt"),
            )
            .add_item(
                BoardItem::new("Stale Drafts", "2 files")
                    .with_status(BoardStatus::Warn)
                    .with_detail("2 drafts older than 30 days"),
            )
            .add_item(BoardItem::new("Broken Schemas", "1").with_status(BoardStatus::Fail))
            .add_item(BoardItem::new("Total Documents", "40"))
            .with_summary("Vault readiness summary across 40 documents.");

        let json = pane.to_json_pretty().unwrap();
        let parsed = BoardPane::from_json_str(&json).unwrap();
        assert_eq!(pane, parsed);
        assert_eq!(parsed.schema_version, CURRENT_BOARD_SCHEMA_VERSION);
        assert_eq!(parsed.title, "Readiness & Quality");
        assert_eq!(
            parsed.gauge,
            Some(BoardGauge {
                label: "Overall Score".to_string(),
                percent: 92,
            })
        );
        assert_eq!(parsed.items.len(), 4);
        assert_eq!(parsed.items[0].status, BoardStatus::Pass);
        assert_eq!(parsed.items[1].status, BoardStatus::Warn);
        assert_eq!(parsed.items[2].status, BoardStatus::Fail);
        assert_eq!(parsed.items[3].status, BoardStatus::Info);
    }

    #[test]
    fn gauge_clamps_percent_to_100() {
        let normal = BoardGauge::new("Progress", 75);
        assert_eq!(normal.percent, 75);

        let boundary = BoardGauge::new("Complete", 100);
        assert_eq!(boundary.percent, 100);

        let overflow = BoardGauge::new("Overflow", 150);
        assert_eq!(overflow.percent, 100);

        let max_u8 = BoardGauge::new("Max", u8::MAX);
        assert_eq!(max_u8.percent, 100);
    }

    #[test]
    fn omits_optional_empty_fields() {
        let pane = BoardPane::new("Minimal Pane");
        let json = pane.to_json().unwrap();
        assert!(!json.contains("gauge"), "{json}");
        assert!(!json.contains("items"), "{json}");
        assert!(!json.contains("summary"), "{json}");

        let parsed = BoardPane::from_json_str(&json).unwrap();
        assert_eq!(pane, parsed);
        assert!(parsed.gauge.is_none());
        assert!(parsed.items.is_empty());
        assert!(parsed.summary.is_none());

        let item = BoardItem::new("Count", "10");
        let item_json = serde_json::to_string(&item).unwrap();
        assert!(!item_json.contains("detail"), "{item_json}");
        assert!(item_json.contains(r#""status":"info""#), "{item_json}");
    }

    #[test]
    fn deserializes_defaults_and_preserves_unknown_extra_fields() {
        let json = r#"{
            "title": "Forward Compatible",
            "future_metric": { "score": 99 },
            "items": [
                {
                    "label": "Default Status Item",
                    "value": "OK"
                }
            ]
        }"#;

        let parsed = BoardPane::from_reader(json.as_bytes()).unwrap();
        assert_eq!(parsed.schema_version, CURRENT_BOARD_SCHEMA_VERSION);
        assert_eq!(parsed.title, "Forward Compatible");
        assert_eq!(parsed.items.len(), 1);
        assert_eq!(parsed.items[0].status, BoardStatus::Info);
        assert_eq!(parsed.items[0].detail, None);
        assert_eq!(
            parsed.extra.get("future_metric"),
            Some(&serde_json::json!({ "score": 99 }))
        );

        let mut buf = Vec::new();
        parsed.print_to(&mut buf).unwrap();
        let printed = String::from_utf8(buf).unwrap();
        let reparsed = BoardPane::from_json_str(&printed).unwrap();
        assert_eq!(parsed, reparsed);
    }
}

//! State types for the interactive Board Layout Editor modal.

use sisho_board_config::BoardConfig;

/// Maximum number of snapshots kept in the layout editor undo history.
pub const MAX_LAYOUT_UNDO_HISTORY: usize = 20;

/// Active column cursor inside the Layout Editor modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EditorColumn {
    #[default]
    Left,
    Right,
    Available,
}

/// Destructive or bulk action awaiting user confirmation in the Layout Editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorConfirm {
    DeleteView,
    ResetDefault,
}

/// Interactive state for the Board Layout Editor modal.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LayoutEditorState {
    pub is_open: bool,
    pub column: EditorColumn,
    pub selected: usize,
    pub grabbed: bool,
    pub rename_buffer: Option<String>,
    pub confirm_action: Option<EditorConfirm>,
    pub undo_stack: Vec<BoardConfig>,
}

impl LayoutEditorState {
    /// Push a configuration snapshot onto the bounded undo stack.
    pub fn push_undo(&mut self, snapshot: BoardConfig) {
        if self.undo_stack.len() >= MAX_LAYOUT_UNDO_HISTORY {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(snapshot);
    }
}

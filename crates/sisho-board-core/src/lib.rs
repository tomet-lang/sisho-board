//! Core state machine, extension trust delegation, and UI configuration for `sisho-board`.

pub mod config;
pub mod editor;
pub mod extension;
pub mod model;
pub mod tools;
pub mod trust;

pub use sisho_board_health::{health, links};
pub use sisho_board_run as runner;

#[cfg(test)]
mod model_tests;

#[cfg(test)]
pub(crate) fn unique_test_suffix() -> String {
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

pub use config::BoardConfig;
pub use editor::resolve_editor;
pub use model::{ExtensionItem, FocusedPane, Model, PendingTrustPrompt, PreviewSource};
pub use trust::TrustState;

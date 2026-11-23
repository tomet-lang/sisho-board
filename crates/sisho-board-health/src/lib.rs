//! Vault readiness, health analysis, and link integrity reporting for `sisho-board`.

pub mod health;
pub mod links;

pub use health::{HealthReport, PillarMetric, analyze, analyze_with_stats, status_for_percent};
pub use links::{BrokenLink, Position, Report as LinksReport, Unreadable};

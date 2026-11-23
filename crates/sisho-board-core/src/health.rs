//! Vault and repository Readiness & Health (`%`) measurement engine.
//!
//! Evaluates six deterministic pillars across a Tomet vault or repository:
//! 1. **Tomet Coverage**: `.tmt` source files + generated `.md` artifacts vs unmigrated `.md` files.
//! 2. **`::id` Declaration**: share of eligible `.tmt` documents declaring `::id(...)`.
//! 3. **`::kind` & Blueprint Coverage**: share of eligible `.tmt` documents declaring `::kind(...)` or `::blueprint(...)`, enriched with `tomet stats --json`.
//! 4. **Link Health**: unbroken links vs total checked links from `tomet check-links --json`.
//! 5. **Writ & Catasto Governance**: presence of `.writ.tmt`, guarded `@rule(...)` blocks, and `catasto.lock` consistency.
//! 6. **Cleanliness & Freshness**: absence of `@draft` markers and validation errors/warnings, plus documentation vs code freshness.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sisho_board_sdk::{BoardGauge, BoardItem, BoardPane, BoardStatus};

use crate::links;
use crate::runner::Step;
use crate::tools::Tool;

/// Score and diagnostic detail for one readiness pillar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PillarMetric {
    /// Machine-readable identifier (e.g. `"tomet_coverage"`).
    pub key: String,
    /// Human-readable pillar label (e.g. `"Tomet Coverage"`).
    pub name: String,
    /// Number of passing items in this pillar.
    pub passed: usize,
    /// Total items evaluated in this pillar.
    pub total: usize,
    /// Completion percentage (`0..=100`).
    pub percent: u8,
    /// Status badge (`Pass` at 100%, `Warn` at 70..=99%, `Fail` below 70%).
    pub status: BoardStatus,
    /// Short value string shown on the card row (e.g. `"95% (19/20)"`).
    pub value_summary: String,
    /// Multi-line actionable breakdown shown in the Output Preview pane.
    pub detail: String,
}

impl PillarMetric {
    /// Convert this pillar metric into a [`BoardItem`] for rendering on the Board.
    pub fn to_board_item(&self) -> BoardItem {
        BoardItem::new(&self.name, &self.value_summary)
            .with_status(self.status)
            .with_detail(&self.detail)
    }
}

/// Complete Readiness & Health report for a vault or repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthReport {
    /// Overall readiness percentage (`0..=100`), average of the active pillars.
    pub overall_percent: u8,
    /// Pillar 1: Tomet (`.tmt`) vs unmigrated Markdown (`.md`) coverage.
    pub tomet_coverage: PillarMetric,
    /// Pillar 2: `::id(...)` declaration coverage across author-owned `.tmt` documents.
    pub id_coverage: PillarMetric,
    /// Pillar 3: `::kind(...)` / `::blueprint(...)` declaration coverage and corpus stats.
    pub kind_coverage: PillarMetric,
    /// Pillar 4: Link integrity from `tomet check-links --json`.
    pub link_health: PillarMetric,
    /// Pillar 5: `.writ.tmt` rule guards and `catasto.lock` governance.
    pub governance: PillarMetric,
    /// Pillar 6: Document cleanliness (no `@draft` or check warnings/failures) and freshness.
    pub cleanliness: PillarMetric,
}

impl HealthReport {
    /// All six pillars in display order.
    pub fn pillars(&self) -> [&PillarMetric; 6] {
        [
            &self.tomet_coverage,
            &self.id_coverage,
            &self.kind_coverage,
            &self.link_health,
            &self.governance,
            &self.cleanliness,
        ]
    }

    /// Convert this report into a [`BoardPane`] for display on `sisho-board`.
    pub fn to_board_pane(&self) -> BoardPane {
        let mut summary_lines = vec![format!(
            "Overall Vault Readiness: {}%",
            self.overall_percent
        )];
        for pillar in self.pillars() {
            summary_lines.push(format!("- {}: {}", pillar.name, pillar.value_summary));
        }

        BoardPane::new("Vault Readiness & Health")
            .with_gauge(BoardGauge::new("Overall Readiness", self.overall_percent))
            .with_items(self.pillars().into_iter().map(PillarMetric::to_board_item))
            .with_summary(summary_lines.join("\n"))
    }

    /// Update the `link_health` pillar and recalculate `overall_percent` when a
    /// link check completes.
    pub fn update_link_health(&mut self, links_report: Option<&links::Report>) {
        self.link_health = measure_link_health(links_report);
        let pillars = self.pillars();
        let sum: u32 = pillars.iter().map(|p| u32::from(p.percent)).sum();
        self.overall_percent = ((sum + (pillars.len() as u32 / 2)) / (pillars.len() as u32)) as u8;
    }

    /// Format a human-readable terminal report with the overall `%` gauge bar,
    /// pillar summary rows, and actionable findings.
    pub fn render_text_report(&self) -> String {
        let clamped = self.overall_percent.min(100) as usize;
        let filled = (clamped * 10 + 50) / 100;
        let empty = 10usize.saturating_sub(filled);
        let bar = format!(
            "[{}{}] {:>3}%",
            "█".repeat(filled),
            "░".repeat(empty),
            clamped
        );

        let mut lines = vec![
            "Vault Readiness & Health".to_string(),
            format!("  {bar} Overall Readiness"),
            String::new(),
            "Pillars:".to_string(),
        ];

        for pillar in self.pillars() {
            let icon = match pillar.status {
                BoardStatus::Pass => "✓",
                BoardStatus::Warn => "!",
                BoardStatus::Fail => "✗",
                BoardStatus::Info => "•",
            };
            lines.push(format!(
                "  {icon} {:<28} {}",
                format!("{}:", pillar.name),
                pillar.value_summary
            ));
        }

        for pillar in self.pillars() {
            if pillar.status != BoardStatus::Pass {
                lines.push(String::new());
                lines.push(format!("--- {} ---", pillar.name));
                lines.push(pillar.detail.clone());
            }
        }

        lines.push(String::new());
        lines.join("\n")
    }
}

/// Status threshold for a percentage (`100` -> `Pass`, `70..=99` -> `Warn`, `<70` -> `Fail`).
pub fn status_for_percent(percent: u8) -> BoardStatus {
    if percent == 100 {
        BoardStatus::Pass
    } else if percent >= 70 {
        BoardStatus::Warn
    } else {
        BoardStatus::Fail
    }
}

fn compute_percent(passed: usize, total: usize, empty_is_hundred: bool) -> u8 {
    if total == 0 {
        if empty_is_hundred { 100 } else { 0 }
    } else {
        ((passed.min(total) * 100) / total) as u8
    }
}

/// Analyze the vault/repository at `dir`, invoking `tomet stats --json` if `tomet` is available.
pub fn analyze(dir: &Path, links_report: Option<&links::Report>) -> HealthReport {
    let stats_outcome = Step::new("tomet stats --json", Tool::Tomet.name())
        .args(["stats", "--json"])
        .arg(dir)
        .run();
    let stats_json = stats_outcome
        .is_pass()
        .then_some(stats_outcome.stdout.as_str());
    analyze_with_stats(dir, links_report, stats_json)
}

/// Analyze the vault/repository at `dir` with an optional pre-fetched `tomet stats --json` payload.
pub fn analyze_with_stats(
    dir: &Path,
    links_report: Option<&links::Report>,
    tomet_stats_json: Option<&str>,
) -> HealthReport {
    let scan = RepoScan::collect(dir);
    let stats_val = tomet_stats_json.and_then(|s| serde_json::from_str::<Value>(s.trim()).ok());

    let tomet_coverage = measure_tomet_coverage(&scan);
    let id_coverage = measure_id_coverage(&scan);
    let kind_coverage = measure_kind_coverage(&scan, stats_val.as_ref());
    let link_health = measure_link_health(links_report);
    let governance = measure_governance(dir, &scan);
    let cleanliness = measure_cleanliness(&scan, stats_val.as_ref());

    let pillars = [
        &tomet_coverage,
        &id_coverage,
        &kind_coverage,
        &link_health,
        &governance,
        &cleanliness,
    ];
    let sum: u32 = pillars.iter().map(|p| u32::from(p.percent)).sum();
    let overall_percent = ((sum + (pillars.len() as u32 / 2)) / (pillars.len() as u32)) as u8;

    HealthReport {
        overall_percent,
        tomet_coverage,
        id_coverage,
        kind_coverage,
        link_health,
        governance,
        cleanliness,
    }
}

#[derive(Debug)]
struct TmtFileInfo {
    rel_path: String,
    has_id: bool,
    has_kind_or_blueprint: bool,
    has_draft: bool,
    is_writ: bool,
    is_catasto_managed: bool,
}

#[derive(Debug)]
struct MdFileInfo {
    rel_path: String,
    is_generated: bool,
}

#[derive(Debug, Default)]
struct RepoScan {
    tmt_files: Vec<TmtFileInfo>,
    md_files: Vec<MdFileInfo>,
    writ_files: Vec<(String, String)>,
    has_catasto_ctmt: bool,
    has_catasto_lock: bool,
    has_cargo_toml: bool,
    latest_code_mtime: Option<SystemTime>,
    latest_doc_mtime: Option<SystemTime>,
}

impl RepoScan {
    fn collect(root: &Path) -> Self {
        let mut scan = Self {
            has_catasto_ctmt: root.join("catasto.ctmt").is_file(),
            has_catasto_lock: root.join("catasto.lock").is_file(),
            has_cargo_toml: root.join("Cargo.toml").is_file(),
            ..Self::default()
        };

        let catasto_managed = parse_catasto_managed_paths(root);
        let mut raw_md_files: Vec<(String, String)> = Vec::new();
        let mut exported_targets = BTreeSet::new();

        walk_dir(
            root,
            root,
            &catasto_managed,
            &mut scan,
            &mut raw_md_files,
            &mut exported_targets,
        );

        for (rel_path, contents) in raw_md_files {
            let file_name = Path::new(&rel_path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            let header_has_generated = contents
                .lines()
                .take(5)
                .any(|line| line.contains("Generated from ") || line.contains("<!-- Generated"));
            let is_generated = file_name == "CLAUDE.md"
                || header_has_generated
                || exported_targets.contains(&rel_path);

            scan.md_files.push(MdFileInfo {
                rel_path,
                is_generated,
            });
        }

        scan
    }
}

fn should_skip_dir(name: &str) -> bool {
    matches!(
        name,
        ".git" | ".jj" | ".direnv" | ".agents" | ".github" | "target" | "node_modules"
    ) || name.starts_with("result")
}

fn is_generated_or_vocab_rel_path(rel: &str) -> bool {
    rel.starts_with("tmtroot/generated/")
        || rel == "tmtroot/generated"
        || rel.starts_with("tmtroot/schema/")
        || rel == "tmtroot/schema"
}

fn walk_dir(
    root: &Path,
    current: &Path,
    catasto_managed: &BTreeSet<String>,
    scan: &mut RepoScan,
    raw_md_files: &mut Vec<(String, String)>,
    exported_targets: &mut BTreeSet<String>,
) {
    let Ok(read_dir) = fs::read_dir(current) else {
        return;
    };
    let mut entries: Vec<_> = read_dir.flatten().collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();
        let file_name = entry.file_name();
        let Some(name_str) = file_name.to_str() else {
            continue;
        };

        let rel_path = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");

        if path.is_dir() {
            if should_skip_dir(name_str) || is_generated_or_vocab_rel_path(&rel_path) {
                continue;
            }
            walk_dir(
                root,
                &path,
                catasto_managed,
                scan,
                raw_md_files,
                exported_targets,
            );
            continue;
        }

        if !path.is_file() || is_generated_or_vocab_rel_path(&rel_path) {
            continue;
        }

        let mtime = entry.metadata().ok().and_then(|m| m.modified().ok());

        if name_str.ends_with(".rs") || name_str == "Cargo.toml" || name_str.ends_with(".nix") {
            update_max_time(&mut scan.latest_code_mtime, mtime);
        }

        if name_str.ends_with(".tmt") {
            update_max_time(&mut scan.latest_doc_mtime, mtime);
            let Ok(contents) = fs::read_to_string(&path) else {
                continue;
            };
            let is_writ = name_str == ".writ.tmt" || name_str.ends_with(".writ.tmt");
            if is_writ {
                scan.writ_files.push((rel_path.clone(), contents.clone()));
            }

            let exported_paths = extract_export_paths(&contents);
            for target in &exported_paths {
                exported_targets.insert(target.clone());
            }

            let (has_id, has_kind_or_blueprint, has_draft) = inspect_tmt_declarations(&contents);
            let is_catasto_managed = catasto_managed.contains(&rel_path);

            scan.tmt_files.push(TmtFileInfo {
                rel_path,
                has_id,
                has_kind_or_blueprint,
                has_draft,
                is_writ,
                is_catasto_managed,
            });
        } else if name_str.ends_with(".md") {
            let Ok(contents) = fs::read_to_string(&path) else {
                continue;
            };
            raw_md_files.push((rel_path, contents));
        }
    }
}

fn update_max_time(slot: &mut Option<SystemTime>, candidate: Option<SystemTime>) {
    if let Some(t) = candidate {
        *slot = Some(match *slot {
            Some(prev) if prev >= t => prev,
            _ => t,
        });
    }
}

fn inspect_tmt_declarations(contents: &str) -> (bool, bool, bool) {
    let mut has_id = false;
    let mut has_kind_or_blueprint = false;
    let mut has_draft = false;

    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") || trimmed.starts_with("``") {
            continue;
        }
        if trimmed.starts_with("::id(") || trimmed.contains(" ::id(") {
            has_id = true;
        }
        if trimmed.starts_with("::kind(")
            || trimmed.contains(" ::kind(")
            || trimmed.starts_with("::blueprint(")
            || trimmed.contains(" ::blueprint(")
        {
            has_kind_or_blueprint = true;
        }
        if trimmed.contains("@draft") {
            has_draft = true;
        }
    }

    (has_id, has_kind_or_blueprint, has_draft)
}

fn extract_export_paths(contents: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let mut in_export = false;
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("::export(") {
            in_export = true;
        }
        if in_export {
            if let Some(rest) = trimmed.strip_prefix("path:") {
                let val = rest.trim().trim_matches(',').trim().trim_matches('"');
                if !val.is_empty() {
                    paths.push(val.trim_start_matches("./").to_string());
                }
            }
            if trimmed.starts_with(')') || trimmed.ends_with(')') {
                in_export = false;
            }
        }
    }
    paths
}

fn parse_catasto_managed_paths(root: &Path) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    for file_name in ["catasto.ctmt", "catasto.lock"] {
        let Ok(contents) = fs::read_to_string(root.join(file_name)) else {
            continue;
        };
        for line in contents.lines() {
            let trimmed = line.trim();
            // In catasto.ctmt / catasto.lock, entries look like:
            //   "tmtroot/dev/agent/behavior.tmt": (...)
            //   ".editorconfig": (...)
            if let Some(rest) = trimmed.strip_prefix('"')
                && let Some((key, after)) = rest.split_once('"')
                && after.trim_start().starts_with(':')
            {
                set.insert(key.trim_start_matches("./").to_string());
            }
        }
    }
    set
}

fn measure_tomet_coverage(scan: &RepoScan) -> PillarMetric {
    let tmt_count = scan.tmt_files.len();
    let generated_md: Vec<&str> = scan
        .md_files
        .iter()
        .filter(|m| m.is_generated)
        .map(|m| m.rel_path.as_str())
        .collect();
    let unmigrated_md: Vec<&str> = scan
        .md_files
        .iter()
        .filter(|m| !m.is_generated)
        .map(|m| m.rel_path.as_str())
        .collect();

    let passed = tmt_count + generated_md.len();
    let total = passed + unmigrated_md.len();
    let percent = compute_percent(passed, total, true);
    let status = status_for_percent(percent);
    let value_summary = format!("{percent}% ({passed}/{total} files)");

    let mut detail_lines = vec![
        format!("Tomet (.tmt) source files: {tmt_count}"),
        format!("Generated/managed .md artifacts: {}", generated_md.len()),
        format!("Unmigrated hand-written .md files: {}", unmigrated_md.len()),
    ];

    if unmigrated_md.is_empty() {
        detail_lines.push(String::new());
        detail_lines.push(
            "✓ All Markdown files are generated artifacts or migrated to Tomet (.tmt)."
                .to_string(),
        );
    } else {
        detail_lines.push(String::new());
        detail_lines.push("Unmigrated Markdown files (migrate with `sisho desk`):".to_string());
        for path in unmigrated_md {
            detail_lines.push(format!("  - {path}"));
        }
    }

    PillarMetric {
        key: "tomet_coverage".to_string(),
        name: "Tomet Coverage".to_string(),
        passed,
        total,
        percent,
        status,
        value_summary,
        detail: detail_lines.join("\n"),
    }
}

fn measure_id_coverage(scan: &RepoScan) -> PillarMetric {
    // Eligible author-owned .tmt documents (excluding .writ.tmt and catasto-managed upstream files).
    let eligible: Vec<&TmtFileInfo> = scan
        .tmt_files
        .iter()
        .filter(|f| !f.is_writ && !f.is_catasto_managed)
        .collect();

    let total = eligible.len();
    let missing: Vec<&str> = eligible
        .iter()
        .filter(|f| !f.has_id)
        .map(|f| f.rel_path.as_str())
        .collect();
    let passed = total.saturating_sub(missing.len());
    let percent = compute_percent(passed, total, true);
    let status = status_for_percent(percent);
    let value_summary = format!("{percent}% ({passed}/{total} docs)");

    let mut detail_lines = vec![format!(
        "Author-owned .tmt documents declaring ::id(...): {passed}/{total}"
    )];
    if missing.is_empty() {
        detail_lines.push(String::new());
        detail_lines.push("✓ All eligible .tmt documents declare ::id(...).".to_string());
    } else {
        detail_lines.push(String::new());
        detail_lines.push("Documents missing ::id(...):".to_string());
        for path in missing {
            detail_lines.push(format!("  - {path}"));
        }
    }

    PillarMetric {
        key: "id_coverage".to_string(),
        name: "::id Declaration".to_string(),
        passed,
        total,
        percent,
        status,
        value_summary,
        detail: detail_lines.join("\n"),
    }
}

fn measure_kind_coverage(scan: &RepoScan, stats_val: Option<&Value>) -> PillarMetric {
    let eligible: Vec<&TmtFileInfo> = scan
        .tmt_files
        .iter()
        .filter(|f| !f.is_catasto_managed)
        .collect();

    let total = eligible.len();
    let missing: Vec<&str> = eligible
        .iter()
        .filter(|f| !f.has_kind_or_blueprint)
        .map(|f| f.rel_path.as_str())
        .collect();
    let passed = total.saturating_sub(missing.len());
    let percent = compute_percent(passed, total, true);
    let status = status_for_percent(percent);
    let value_summary = format!("{percent}% ({passed}/{total} typed)");

    let mut detail_lines = vec![format!(
        "Author-owned .tmt documents declaring ::kind(...) or ::blueprint(...): {passed}/{total}"
    )];

    if missing.is_empty() {
        detail_lines.push(String::new());
        detail_lines.push(
            "✓ All eligible .tmt documents declare ::kind(...) or ::blueprint(...).".to_string(),
        );
    } else {
        detail_lines.push(String::new());
        detail_lines.push("Documents missing ::kind(...) or ::blueprint(...):".to_string());
        for path in missing {
            detail_lines.push(format!("  - {path}"));
        }
    }

    if let Some(val) = stats_val {
        detail_lines.push(String::new());
        detail_lines.push("--- tomet stats ---".to_string());
        let files = val.get("files").and_then(Value::as_u64).unwrap_or(0);
        let chars = val.get("characters").and_then(Value::as_u64).unwrap_or(0);
        let headings = val
            .get("headings")
            .and_then(|h| h.get("count"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let elements_total = val
            .get("elements")
            .and_then(|e| e.get("total"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        detail_lines.push(format!(
            "Corpus: {files} file(s), {chars} chars, {headings} headings, {elements_total} elements"
        ));

        if let Some(kinds) = val.get("kinds").and_then(Value::as_array) {
            detail_lines.push("By kind:".to_string());
            for entry in kinds {
                let kind_label = entry
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("(untyped)");
                let k_files = entry.get("files").and_then(Value::as_u64).unwrap_or(0);
                let k_chars = entry.get("characters").and_then(Value::as_u64).unwrap_or(0);
                detail_lines.push(format!("  - {kind_label}: {k_files} file(s), {k_chars} chars"));
            }
        }
    }

    PillarMetric {
        key: "kind_coverage".to_string(),
        name: "::kind & Blueprint".to_string(),
        passed,
        total,
        percent,
        status,
        value_summary,
        detail: detail_lines.join("\n"),
    }
}

fn measure_link_health(links_report: Option<&links::Report>) -> PillarMetric {
    let Some(report) = links_report else {
        return PillarMetric {
            key: "link_health".to_string(),
            name: "Link Health".to_string(),
            passed: 0,
            total: 0,
            percent: 100,
            status: BoardStatus::Info,
            value_summary: "100% (scanning...)".to_string(),
            detail: "Link verification is running or has not been provided yet.".to_string(),
        };
    };

    let broken_count = report.broken.len();
    let unreadable_count = report.unreadable.len();
    let total = report.links_checked + unreadable_count;
    let passed = report.links_checked.saturating_sub(broken_count);
    let percent = if unreadable_count > 0 && total == 0 {
        0
    } else {
        compute_percent(passed, total, true)
    };
    let status = if unreadable_count > 0 || broken_count > 0 {
        if percent >= 70 && unreadable_count == 0 {
            BoardStatus::Warn
        } else {
            BoardStatus::Fail
        }
    } else {
        BoardStatus::Pass
    };
    let value_summary = format!("{percent}% ({passed}/{total} links)");

    let mut detail_lines = vec![format!(
        "Scanned {} file(s), checked {} link(s).",
        report.files_scanned, report.links_checked
    )];

    if broken_count == 0 && unreadable_count == 0 {
        detail_lines.push(String::new());
        detail_lines.push("✓ No broken links or unreadable documents found.".to_string());
    } else {
        if broken_count > 0 {
            detail_lines.push(String::new());
            detail_lines.push(format!("Broken links ({broken_count}):"));
            for link in &report.broken {
                let loc = link.at.map(|p| format!(":{}", p.line)).unwrap_or_default();
                detail_lines.push(format!(
                    "  - {}{loc} -> {}",
                    link.source.display(),
                    link.target
                ));
            }
        }
        if unreadable_count > 0 {
            detail_lines.push(String::new());
            detail_lines.push(format!("Unreadable documents ({unreadable_count}):"));
            for err in &report.unreadable {
                detail_lines.push(format!("  - {}: {}", err.source.display(), err.error));
            }
        }
    }

    PillarMetric {
        key: "link_health".to_string(),
        name: "Link Health".to_string(),
        passed,
        total,
        percent,
        status,
        value_summary,
        detail: detail_lines.join("\n"),
    }
}

fn measure_governance(_dir: &Path, scan: &RepoScan) -> PillarMetric {
    if scan.writ_files.is_empty() {
        return PillarMetric {
            key: "governance".to_string(),
            name: "Writ & Catasto Governance".to_string(),
            passed: 0,
            total: 1,
            percent: 0,
            status: BoardStatus::Fail,
            value_summary: "0% (no .writ.tmt)".to_string(),
            detail: "No .writ.tmt found in repository.\nAdd a .writ.tmt with guarded @rule(...) blocks to enforce architectural invariants with twrit.".to_string(),
        };
    }

    let mut rules: Vec<(String, String, bool)> = Vec::new();
    let mut has_rust_api_guard = false;

    for (writ_path, contents) in &scan.writ_files {
        for (rule_name, has_guard, is_rust_api) in parse_writ_rules(contents) {
            if is_rust_api {
                has_rust_api_guard = true;
            }
            rules.push((writ_path.clone(), rule_name, has_guard));
        }
    }

    let mut passed = 0usize;
    let mut total = 0usize;
    let mut detail_lines = Vec::new();

    if rules.is_empty() {
        total += 1;
        detail_lines.push("✗ .writ.tmt exists but declares no @rule(...) blocks.".to_string());
    } else {
        detail_lines.push(format!("Writ rules discovered ({}):", rules.len()));
        for (writ_path, rule_name, has_guard) in &rules {
            total += 1;
            if *has_guard {
                passed += 1;
                detail_lines.push(format!("  ✓ @rule({rule_name}) in {writ_path} (guarded)"));
            } else {
                detail_lines.push(format!(
                    "  ✗ @rule({rule_name}) in {writ_path} (missing guard:)"
                ));
            }
        }
    }

    if scan.has_catasto_ctmt {
        total += 1;
        detail_lines.push(String::new());
        if scan.has_catasto_lock {
            passed += 1;
            detail_lines
                .push("✓ catasto.ctmt is paired with catasto.lock.".to_string());
        } else {
            detail_lines.push(
                "✗ catasto.ctmt exists without catasto.lock (run `sisho catasto sync`)."
                    .to_string(),
            );
        }
    }

    if scan.has_cargo_toml {
        detail_lines.push(String::new());
        if has_rust_api_guard {
            detail_lines.push(
                "✓ Rust workspace has `twrit: rust-api` API doc extraction configured."
                    .to_string(),
            );
        } else {
            detail_lines.push(
                "• Rust workspace detected (`Cargo.toml`); consider adding a `twrit: rust-api` guard."
                    .to_string(),
            );
        }
    }

    let percent = compute_percent(passed, total, false);
    let status = status_for_percent(percent);
    let value_summary = format!("{percent}% ({passed}/{total} guards)");

    PillarMetric {
        key: "governance".to_string(),
        name: "Writ & Catasto Governance".to_string(),
        passed,
        total,
        percent,
        status,
        value_summary,
        detail: detail_lines.join("\n"),
    }
}

fn parse_writ_rules(contents: &str) -> Vec<(String, bool, bool)> {
    let mut rules = Vec::new();
    let mut current_rule: Option<(String, bool, bool, i32)> = None;

    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") {
            continue;
        }
        if current_rule.is_none()
            && let Some(pos) = trimmed.find("@rule(")
        {
            let after = &trimmed[pos + "@rule(".len()..];
            let name = after
                .split(')')
                .next()
                .unwrap_or("unnamed")
                .trim()
                .to_string();
            let depth = trimmed.chars().filter(|&c| c == '{').count() as i32
                - trimmed.chars().filter(|&c| c == '}').count() as i32;
            let has_guard = trimmed.contains("guard:");
            let is_rust_api = trimmed.contains("rust-api");
            if depth <= 0 && trimmed.contains('}') {
                rules.push((name, has_guard, is_rust_api));
            } else {
                current_rule = Some((name, has_guard, is_rust_api, depth.max(1)));
            }
            continue;
        }

        if let Some((name, mut has_guard, mut is_rust_api, mut depth)) = current_rule.take() {
            if trimmed.contains("guard:") {
                has_guard = true;
            }
            if trimmed.contains("rust-api") {
                is_rust_api = true;
            }
            depth += trimmed.chars().filter(|&c| c == '{').count() as i32;
            depth -= trimmed.chars().filter(|&c| c == '}').count() as i32;
            if depth <= 0 {
                rules.push((name, has_guard, is_rust_api));
            } else {
                current_rule = Some((name, has_guard, is_rust_api, depth));
            }
        }
    }

    if let Some((name, has_guard, is_rust_api, _)) = current_rule {
        rules.push((name, has_guard, is_rust_api));
    }

    rules
}

fn measure_cleanliness(scan: &RepoScan, stats_val: Option<&Value>) -> PillarMetric {
    let total = scan.tmt_files.len();
    let draft_files: Vec<&str> = scan
        .tmt_files
        .iter()
        .filter(|f| f.has_draft)
        .map(|f| f.rel_path.as_str())
        .collect();

    let stats_failed = stats_val
        .and_then(|v| v.get("check"))
        .and_then(|c| c.get("failed"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    let stats_warnings = stats_val
        .and_then(|v| v.get("check"))
        .and_then(|c| c.get("warnings"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;

    let deductions = draft_files.len() + stats_failed + stats_warnings;
    let passed = total.saturating_sub(deductions);
    let percent = compute_percent(passed, total, true);
    let status = status_for_percent(percent);
    let value_summary = format!("{percent}% ({passed}/{total} clean)");

    let mut detail_lines = vec![format!("Clean .tmt documents: {passed}/{total}")];

    if draft_files.is_empty() {
        detail_lines.push("✓ No @draft markers found in .tmt documents.".to_string());
    } else {
        detail_lines.push(String::new());
        detail_lines.push(format!("Documents containing @draft ({}):", draft_files.len()));
        for path in draft_files {
            detail_lines.push(format!("  - {path}"));
        }
    }

    if stats_failed > 0 || stats_warnings > 0 {
        detail_lines.push(format!(
            "✗ tomet check diagnostics: {stats_failed} failed, {stats_warnings} warning(s)."
        ));
    }

    if let (Some(code_t), Some(doc_t)) = (scan.latest_code_mtime, scan.latest_doc_mtime) {
        detail_lines.push(String::new());
        if doc_t >= code_t {
            detail_lines.push(
                "✓ Documentation freshness: .tmt files are up-to-date with recent code changes."
                    .to_string(),
            );
        } else {
            detail_lines.push(
                "• Documentation freshness: source code was modified more recently than .tmt documents."
                    .to_string(),
            );
        }
    }

    PillarMetric {
        key: "cleanliness".to_string(),
        name: "Cleanliness & Freshness".to_string(),
        passed,
        total,
        percent,
        status,
        value_summary,
        detail: detail_lines.join("\n"),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::links::{BrokenLink, Position, Report as LinksReport};

    fn temp_fixture_dir(slug: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sisho-board-health-test-{}-{slug}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn well_formed_vault_scores_100_percent_across_all_pillars() {
        let dir = temp_fixture_dir("perfect");
        fs::create_dir_all(dir.join("tmtroot/spec")).unwrap();

        fs::write(
            dir.join(".writ.tmt"),
            "::kind(writ)\n\n@rule(layering){\n  guard: (twrit: layers)\n}\n",
        )
        .unwrap();
        fs::write(
            dir.join("catasto.ctmt"),
            "::kind(catasto)\n::files(\n  \".editorconfig\": ()\n)\n",
        )
        .unwrap();
        fs::write(dir.join("catasto.lock"), "{}\n").unwrap();

        fs::write(
            dir.join("tmtroot/readme.tmt"),
            "::id(doc-readme)\n::kind(guide)\n::export(\n  type: commonmark\n  path: \"README.md\"\n)\n=[ Title ]\n",
        )
        .unwrap();
        fs::write(
            dir.join("tmtroot/spec/overview.tmt"),
            "::id(spec.overview)\n::kind(spec)\n=[ Overview ]\n",
        )
        .unwrap();
        fs::write(
            dir.join("README.md"),
            "<!-- Generated from tmtroot/readme.tmt. Edit that, then `tomet export .`. -->\n# Title\n",
        )
        .unwrap();
        fs::write(dir.join("CLAUDE.md"), "@AGENTS.md\n").unwrap();

        let links_report = LinksReport {
            files_scanned: 3,
            links_checked: 10,
            broken: Vec::new(),
            unreadable: Vec::new(),
        };

        let stats_json = r#"{
            "files": 3,
            "characters": 250,
            "elements": { "total": 12 },
            "headings": { "count": 2, "max_level": 1 },
            "check": { "failed": 0, "warnings": 0 },
            "kinds": [
                { "kind": "writ", "files": 1, "characters": 50 },
                { "kind": "guide", "files": 1, "characters": 100 },
                { "kind": "spec", "files": 1, "characters": 100 }
            ]
        }"#;

        let report = analyze_with_stats(&dir, Some(&links_report), Some(stats_json));
        assert_eq!(report.overall_percent, 100);
        for pillar in report.pillars() {
            assert_eq!(pillar.percent, 100, "pillar {} was not 100%", pillar.name);
            assert_eq!(pillar.status, BoardStatus::Pass);
        }

        let pane = report.to_board_pane();
        assert_eq!(pane.title, "Vault Readiness & Health");
        assert_eq!(pane.gauge.as_ref().unwrap().percent, 100);
        assert_eq!(pane.items.len(), 6);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn partially_migrated_vault_identifies_unmigrated_md_missing_id_kind_draft_and_broken_links() {
        let dir = temp_fixture_dir("partial");
        fs::create_dir_all(dir.join("tmtroot")).unwrap();

        // 1. .writ.tmt with 1 guarded rule and 1 unguarded rule (50%)
        fs::write(
            dir.join(".writ.tmt"),
            "::kind(writ)\n@rule(guarded-rule){\n  guard: (twrit: layers)\n}\n@rule(manual-rule){\n  note: \"no guard\"\n}\n",
        )
        .unwrap();

        // 2. One complete .tmt file and one incomplete .tmt file (missing ::id, missing ::kind, has @draft)
        fs::write(
            dir.join("tmtroot/good.tmt"),
            "::id(doc-good)\n::kind(spec)\n=[ Good ]\n",
        )
        .unwrap();
        fs::write(
            dir.join("tmtroot/unfinished.tmt"),
            "=[ Unfinished ]\n@draft[Still writing this section]\n",
        )
        .unwrap();

        // 3. One unmigrated Markdown file
        fs::write(dir.join("notes.md"), "# Hand-written legacy notes\n").unwrap();

        // 4. Broken link in links::Report (3/4 links valid = 75%)
        let links_report = LinksReport {
            files_scanned: 3,
            links_checked: 4,
            broken: vec![BrokenLink {
                source: PathBuf::from("tmtroot/unfinished.tmt"),
                target: "missing.tmt".to_string(),
                at: Some(Position { line: 2, column: 1 }),
            }],
            unreadable: Vec::new(),
        };

        let report = analyze_with_stats(&dir, Some(&links_report), None);

        // Tomet Coverage: 3 .tmt files (.writ.tmt, good.tmt, unfinished.tmt) vs 1 unmigrated notes.md -> 3/4 = 75%
        assert_eq!(report.tomet_coverage.passed, 3);
        assert_eq!(report.tomet_coverage.total, 4);
        assert_eq!(report.tomet_coverage.percent, 75);
        assert_eq!(report.tomet_coverage.status, BoardStatus::Warn);
        assert!(
            report.tomet_coverage.detail.contains("notes.md"),
            "{}",
            report.tomet_coverage.detail
        );

        // ::id Declaration: good.tmt has ::id, unfinished.tmt is missing ::id -> 1/2 = 50%
        assert_eq!(report.id_coverage.passed, 1);
        assert_eq!(report.id_coverage.total, 2);
        assert_eq!(report.id_coverage.percent, 50);
        assert_eq!(report.id_coverage.status, BoardStatus::Fail);
        assert!(
            report.id_coverage.detail.contains("tmtroot/unfinished.tmt"),
            "{}",
            report.id_coverage.detail
        );

        // ::kind & Blueprint: .writ.tmt and good.tmt have ::kind, unfinished.tmt is missing ::kind -> 2/3 = 66%
        assert_eq!(report.kind_coverage.passed, 2);
        assert_eq!(report.kind_coverage.total, 3);
        assert_eq!(report.kind_coverage.percent, 66);
        assert_eq!(report.kind_coverage.status, BoardStatus::Fail);
        assert!(
            report.kind_coverage.detail.contains("tmtroot/unfinished.tmt"),
            "{}",
            report.kind_coverage.detail
        );

        // Link Health: 3/4 = 75%
        assert_eq!(report.link_health.passed, 3);
        assert_eq!(report.link_health.total, 4);
        assert_eq!(report.link_health.percent, 75);
        assert_eq!(report.link_health.status, BoardStatus::Warn);
        assert!(
            report.link_health.detail.contains("missing.tmt"),
            "{}",
            report.link_health.detail
        );

        // Governance: 1/2 guarded rules = 50%
        assert_eq!(report.governance.passed, 1);
        assert_eq!(report.governance.total, 2);
        assert_eq!(report.governance.percent, 50);
        assert_eq!(report.governance.status, BoardStatus::Fail);
        assert!(
            report.governance.detail.contains("manual-rule"),
            "{}",
            report.governance.detail
        );

        // Cleanliness: 2/3 clean (unfinished.tmt has @draft) = 66%
        assert_eq!(report.cleanliness.passed, 2);
        assert_eq!(report.cleanliness.total, 3);
        assert_eq!(report.cleanliness.percent, 66);
        assert_eq!(report.cleanliness.status, BoardStatus::Fail);
        assert!(
            report.cleanliness.detail.contains("tmtroot/unfinished.tmt"),
            "{}",
            report.cleanliness.detail
        );

        let _ = fs::remove_dir_all(&dir);
    }
}

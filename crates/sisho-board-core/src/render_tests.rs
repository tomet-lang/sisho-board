use std::path::PathBuf;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use sisho_board_sdk::{BoardGauge, BoardItem, BoardPane, BoardStatus};
use sisho_sdk::ExtensionInfo;

use crate::model::{
    ExtensionItem, FocusedPane, Model, PreviewSource, StepSlot, ToolStatus,
};
use crate::model_tests::sample_health_report;
use crate::runner::{Outcome, Status};
use crate::tools::{Verdict, Version};
use crate::trust::{TrustConfig, TrustState};
use crate::ui;

fn create_mock_model() -> Model {
    let (sender, receiver) = std::sync::mpsc::channel();
    let outcomes = vec![
        Outcome {
            name: "tomet check".to_string(),
            command_line: "tomet check .".to_string(),
            status: Status::Passed,
            stdout: "all passed\nline 2\nline 3\nline 4\nline 5\n".to_string(),
            stderr: String::new(),
            duration: std::time::Duration::from_millis(15),
        },
        Outcome {
            name: "twrit check".to_string(),
            command_line: "twrit check .".to_string(),
            status: Status::Failed { code: 1 },
            stdout: String::new(),
            stderr: "rule error in spec\n".to_string(),
            duration: std::time::Duration::from_millis(90),
        },
    ];
    let check_slots = outcomes.iter().cloned().map(StepSlot::Done).collect();

    let mut desk = ExtensionItem::new("desk", PathBuf::from("/path/to/sisho-desk"), true, None);
    desk.trust = TrustState::Allowed;
    desk.info = Some(ExtensionInfo::new(
        "desk",
        "0.2.0",
        "Interactive desk for Tomet",
    ));

    let untrusted = ExtensionItem::new(
        "experimental",
        PathBuf::from("/path/to/sisho-experimental"),
        true,
        None,
    );

    Model {
        dir: PathBuf::from("."),
        config_path: None,
        trust_config: TrustConfig::new(),
        tools: vec![
            ToolStatus {
                name: "tomet",
                version: Some("0.5.0".to_string()),
                verdict: Verdict::Fine(Version::new(0, 5, 0)),
            },
            ToolStatus {
                name: "twrit",
                version: Some("0.4.0".to_string()),
                verdict: Verdict::Fine(Version::new(0, 4, 0)),
            },
            ToolStatus {
                name: "catasto",
                version: None,
                verdict: Verdict::Unreported {
                    needs: Version::new(0, 1, 0),
                },
            },
        ],
        extensions: vec![desk, untrusted],
        check_slots,
        check_outcomes: outcomes,
        links_report: None,
        health_report: Some(sample_health_report()),
        selected_extension: 0,
        selected_step: 0,
        selected_health_pillar: 0,
        selected_broken_link: 0,
        selected_ext_item: 0,
        preview_scroll: 0,
        focused_pane: FocusedPane::default(),
        preview_source: PreviewSource::default(),
        confirm_fix: false,
        message: None,
        is_checking: false,
        is_checking_links: false,
        is_checking_health: false,
        is_fixing: false,
        tick: 0,
        sender,
        receiver,
    }
}

#[test]
fn renders_normal_board_and_trust_badges() {
    let backend = TestBackend::new(110, 36);
    let mut terminal = Terminal::new(backend).unwrap();

    let model = create_mock_model();
    terminal.draw(|f| ui::draw(f, &model)).unwrap();

    let view = terminal.backend().to_string();
    assert!(view.contains("Sisho Board (Project Status)"), "{view}");
    assert!(view.contains("Tools & Extensions"), "{view}");
    assert!(view.contains("tomet: 0.5.0 (fine)"), "{view}");
    assert!(view.contains("✓ desk v0.2.0 [allowed]"), "{view}");
    assert!(view.contains("Interactive desk for Tomet"), "{view}");
    assert!(
        view.contains("? experimental [untrusted: a/Enter]"),
        "{view}"
    );
    assert!(view.contains("Integrity & Checks"), "{view}");
    assert!(view.contains("Vault Readiness & Health"), "{view}");
    assert!(view.contains("[█████████░]  92% Overall Readiness"), "{view}");
    assert!(view.contains("✓ Tomet Coverage: 100%"), "{view}");
    assert!(view.contains("! ::id Declaration: 80%"), "{view}");
    assert!(view.contains("tomet check"), "{view}");
    assert!(view.contains("Counter (Actions)"), "{view}");
    assert!(view.contains("Pane:Checks"), "{view}");
}

#[test]
fn renders_focused_health_card_and_pillar_detail_in_preview() {
    let backend = TestBackend::new(110, 36);
    let mut terminal = Terminal::new(backend).unwrap();

    let mut model = create_mock_model();
    model.focused_pane = FocusedPane::Health;
    model.preview_source = PreviewSource::HealthPillar;
    model.selected_health_pillar = 1; // ::id Declaration

    terminal.draw(|f| ui::draw(f, &model)).unwrap();

    let view = terminal.backend().to_string();
    assert!(view.contains("Vault Readiness & Health"), "{view}");
    assert!(view.contains("> ! ::id Declaration: 80%"), "{view}");
    assert!(view.contains("Pillar: ::id Declaration — 80%"), "{view}");
    assert!(view.contains("Documents missing ::id(...):"), "{view}");
    assert!(view.contains("- tmtroot/draft.tmt"), "{view}");
    assert!(view.contains("Pane:Health"), "{view}");
}

#[test]
fn renders_extension_board_pane_with_gauge_and_detail_preview() {
    let backend = TestBackend::new(110, 40);
    let mut terminal = Terminal::new(backend).unwrap();

    let mut model = create_mock_model();
    model.extensions[0].board_pane = Some(
        BoardPane::new("Readiness & Quality")
            .with_gauge(BoardGauge::new("Overall Score", 90))
            .add_item(
                BoardItem::new("Metadata Coverage", "95% (38/40)")
                    .with_status(BoardStatus::Pass)
                    .with_detail("Missing id in:\n- docs/draft1.tmt\n- docs/draft2.tmt"),
            )
            .add_item(
                BoardItem::new("Stale Drafts", "2 files").with_status(BoardStatus::Warn),
            ),
    );
    model.focused_pane = FocusedPane::ExtensionPanes;
    model.preview_source = PreviewSource::ExtensionItem;

    terminal.draw(|f| ui::draw(f, &model)).unwrap();

    let view = terminal.backend().to_string();
    assert!(
        view.contains("Readiness & Quality (sisho-desk)"),
        "{view}"
    );
    assert!(view.contains("[█████████░]  90% Overall Score"), "{view}");
    assert!(
        view.contains("> ✓ Metadata Coverage: 95% (38/40)"),
        "{view}"
    );
    assert!(view.contains("! Stale Drafts: 2 files"), "{view}");
    assert!(view.contains("Missing id in:"), "{view}");
    assert!(view.contains("- docs/draft1.tmt"), "{view}");
    assert!(view.contains("Pane:ExtPane"), "{view}");
}

#[test]
fn renders_extensions_pane_focus_and_toggle_action_hint() {
    let backend = TestBackend::new(110, 32);
    let mut terminal = Terminal::new(backend).unwrap();

    let mut model = create_mock_model();
    model.focused_pane = FocusedPane::Extensions;
    model.selected_extension = 1;

    terminal.draw(|f| ui::draw(f, &model)).unwrap();

    let view = terminal.backend().to_string();
    assert!(view.contains("Pane:Extensions"), "{view}");
    assert!(view.contains("[a/Enter]"), "{view}");
    assert!(view.contains("Toggle Trust"), "{view}");
    assert!(
        view.contains("> ? experimental [untrusted: a/Enter]"),
        "{view}"
    );
}

#[test]
fn renders_confirm_fix_prompt() {
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    let mut model = create_mock_model();
    model.confirm_fix = true;
    terminal.draw(|f| ui::draw(f, &model)).unwrap();

    let view = terminal.backend().to_string();
    assert!(
        view.contains("Apply fixes across the project? (writes files)"),
        "{view}"
    );
    assert!(view.contains("[y] Yes, apply fixes"), "{view}");
    assert!(view.contains("[n/Esc] Cancel"), "{view}");
}

#[test]
fn renders_broken_links_list() {
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    let mut model = create_mock_model();
    model.links_report = Some(crate::links::Report {
        files_scanned: 10,
        links_checked: 42,
        broken: vec![crate::links::BrokenLink {
            source: PathBuf::from("docs/intro.tmt"),
            target: "missing.tmt".to_string(),
            at: Some(crate::links::Position {
                line: 15,
                column: 1,
            }),
        }],
        unreadable: Vec::new(),
    });
    model.focused_pane = FocusedPane::BrokenLinks;

    terminal.draw(|f| ui::draw(f, &model)).unwrap();

    let view = terminal.backend().to_string();
    assert!(view.contains("Broken Links (1/1)"), "{view}");
    assert!(view.contains("docs/intro.tmt:15 -> missing.tmt"), "{view}");
    assert!(view.contains("[Enter/e]"), "{view}");
    assert!(view.contains("Edit"), "{view}");
}

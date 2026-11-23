use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use sisho_board_sdk::{BoardGauge, BoardItem, BoardPane, BoardStatus};

use crate::config::BoardConfig;
use crate::health::{HealthReport, PillarMetric};
use crate::links::{BrokenLink, Position, Report};
use crate::model::{
    ExtensionItem, FocusedPane, Model, PendingTrustPrompt, PreviewSource, StepSlot, WorkerMessage,
};
use crate::runner::{Outcome, Status};
use crate::trust::TrustState;

fn sample_pillar(key: &str, name: &str, percent: u8, detail: &str) -> PillarMetric {
    PillarMetric {
        key: key.to_string(),
        name: name.to_string(),
        passed: percent as usize,
        total: 100,
        percent,
        status: crate::health::status_for_percent(percent),
        value_summary: format!("{percent}%"),
        detail: detail.to_string(),
    }
}

pub fn sample_health_report() -> HealthReport {
    HealthReport {
        overall_percent: 92,
        tomet_coverage: sample_pillar(
            "tomet_coverage",
            "Tomet Coverage",
            100,
            "All Markdown files are generated artifacts or migrated to Tomet (.tmt).",
        ),
        id_coverage: sample_pillar(
            "id_coverage",
            "::id Declaration",
            80,
            "Documents missing ::id(...):\n  - tmtroot/draft.tmt",
        ),
        kind_coverage: sample_pillar(
            "kind_coverage",
            "::kind & Blueprint",
            90,
            "Documents missing ::kind(...):\n  - tmtroot/draft.tmt",
        ),
        link_health: sample_pillar(
            "link_health",
            "Link Health",
            100,
            "No broken links found.",
        ),
        governance: sample_pillar(
            "governance",
            "Writ & Catasto Governance",
            100,
            "All 4 rules guarded.",
        ),
        cleanliness: sample_pillar(
            "cleanliness",
            "Cleanliness & Freshness",
            80,
            "Documents containing @draft (1):\n  - tmtroot/draft.tmt",
        ),
    }
}

fn create_test_model() -> Model {
    let (sender, receiver) = std::sync::mpsc::channel();
    Model {
        dir: PathBuf::from("."),
        sisho_program: PathBuf::from("sisho"),
        board_config_path: None,
        board_config: BoardConfig::new(),
        tools: Vec::new(),
        extensions: vec![ExtensionItem::new(
            "desk",
            PathBuf::from("/bin/sisho-desk"),
            true,
            None,
        )],
        check_slots: vec![
            StepSlot::Running {
                name: "step 1".to_string(),
            },
            StepSlot::Running {
                name: "step 2".to_string(),
            },
        ],
        check_outcomes: Vec::new(),
        links_report: None,
        health_report: Some(sample_health_report()),
        selected_extension: 0,
        selected_step: 0,
        selected_health_pillar: 0,
        selected_broken_link: 0,
        selected_ext_item: 0,
        preview_scroll: 0,
        focused_pane: FocusedPane::Checks,
        preview_source: PreviewSource::CheckStep,
        confirm_fix: false,
        pending_trust: None,
        message: None,
        is_checking: true,
        is_checking_links: false,
        is_checking_health: false,
        is_fixing: false,
        tick: 0,
        sender,
        receiver,
    }
}

#[test]
fn individual_step_completion_updates_only_its_slot() {
    let mut model = create_test_model();
    assert_eq!(model.selected_outcome(), None);

    let outcome1 = Outcome {
        name: "step 1".to_string(),
        command_line: "step 1".to_string(),
        status: Status::Passed,
        stdout: "ok".to_string(),
        stderr: String::new(),
        duration: Duration::from_millis(50),
    };

    model
        .sender
        .send(WorkerMessage::CheckStepDone {
            index: 0,
            outcome: outcome1.clone(),
        })
        .unwrap();

    model.poll_background();

    assert_eq!(model.check_slots[0], StepSlot::Done(outcome1.clone()));
    assert_eq!(
        model.check_slots[1],
        StepSlot::Running {
            name: "step 2".to_string()
        }
    );
    assert_eq!(model.selected_outcome(), Some(&outcome1));
}

#[test]
fn pane_focus_cycling_includes_health_and_updates_preview_source() {
    let mut model = create_test_model();
    model.links_report = Some(Report {
        files_scanned: 2,
        links_checked: 2,
        broken: vec![
            BrokenLink {
                source: PathBuf::from("a.tmt"),
                target: "b.tmt".to_string(),
                at: Some(Position { line: 1, column: 1 }),
            },
            BrokenLink {
                source: PathBuf::from("c.tmt"),
                target: "d.tmt".to_string(),
                at: Some(Position { line: 2, column: 1 }),
            },
        ],
        unreadable: Vec::new(),
    });

    // Without extension panes: Checks -> Health -> Extensions -> BrokenLinks -> OutputPreview -> Checks
    assert_eq!(model.focused_pane, FocusedPane::Checks);
    model.move_selection_down();
    assert_eq!(model.selected_step, 1);

    model.next_pane();
    assert_eq!(model.focused_pane, FocusedPane::Health);
    assert_eq!(model.preview_source, PreviewSource::HealthPillar);
    assert_eq!(model.selected_health_pillar, 0);
    assert!(
        model
            .selected_preview_text()
            .unwrap()
            .contains("All Markdown files are generated")
    );

    model.move_selection_down();
    assert_eq!(model.selected_health_pillar, 1);
    assert!(
        model
            .selected_preview_text()
            .unwrap()
            .contains("tmtroot/draft.tmt")
    );

    model.next_pane();
    assert_eq!(model.focused_pane, FocusedPane::Extensions);

    model.next_pane();
    assert_eq!(model.focused_pane, FocusedPane::BrokenLinks);
    model.move_selection_down();
    assert_eq!(model.selected_broken_link, 1);

    model.next_pane();
    assert_eq!(model.focused_pane, FocusedPane::OutputPreview);
    model.move_selection_down();
    assert_eq!(model.preview_scroll, 1);

    model.next_pane();
    assert_eq!(model.focused_pane, FocusedPane::Checks);
    assert_eq!(model.preview_source, PreviewSource::CheckStep);
}

#[test]
fn untrusted_and_denied_extensions_are_never_executed_until_confirmed_allowed() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = std::env::temp_dir().join(format!(
            "sisho-board-sec-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let marker = temp_dir.join("executed.marker");
        let env_log = temp_dir.join("ext_env.log");
        let sisho_log = temp_dir.join("sisho_calls.log");
        let ext_path = temp_dir.join("sisho-audit");
        let mock_sisho = temp_dir.join("sisho");

        let pane_json = BoardPane::new("Vault Audit")
            .with_gauge(BoardGauge::new("Readiness", 88))
            .add_item(
                BoardItem::new("IDs", "44/50")
                    .with_status(BoardStatus::Warn)
                    .with_detail("6 documents missing ::id"),
            )
            .to_json()
            .unwrap();

        let ext_script = format!(
            "#!/bin/sh\n\
             touch \"{}\"\n\
             printf '%s\\n%s\\n' \"$SISHO_CONFIG_DIR\" \"$SISHO_DATA_DIR\" > \"{}\"\n\
             if [ \"$1\" = \"--sisho-extension-info\" ]; then\n\
               echo '{{\"schema_version\":1,\"word\":\"audit\",\"version\":\"0.3.0\",\"about\":\"Audit extension\"}}'\n\
               exit 0\n\
             fi\n\
             if [ \"$1\" = \"--sisho-board-pane\" ]; then\n\
               echo '{}'\n\
               exit 0\n\
             fi\n",
            marker.display(),
            env_log.display(),
            pane_json
        );
        fs::write(&ext_path, ext_script).unwrap();
        fs::set_permissions(&ext_path, fs::Permissions::from_mode(0o755)).unwrap();

        let sisho_script = format!(
            "#!/bin/sh\n\
             echo \"$@\" >> \"{}\"\n\
             exit 0\n",
            sisho_log.display()
        );
        fs::write(&mock_sisho, sisho_script).unwrap();
        fs::set_permissions(&mock_sisho, fs::Permissions::from_mode(0o755)).unwrap();

        let canonical_ext = fs::canonicalize(&ext_path).unwrap();

        let mut model = create_test_model();
        model.sisho_program = mock_sisho.clone();
        model.extensions = vec![ExtensionItem::new("audit", ext_path.clone(), true, None)];
        assert_eq!(model.extensions[0].canonical_path, canonical_ext);

        // 1. Untrusted: query_allowed_extensions must NOT execute the binary
        assert_eq!(model.extensions[0].trust, TrustState::Untrusted);
        model.query_allowed_extensions();
        std::thread::sleep(Duration::from_millis(50));
        model.poll_background();
        assert!(
            !marker.exists(),
            "Untrusted extension must never be executed"
        );
        assert!(model.extensions[0].board_pane.is_none());

        // 2. Opening trust prompt sets pending_trust WITHOUT executing or modifying trust
        model.request_trust_prompt();
        assert_eq!(
            model.pending_trust,
            Some(PendingTrustPrompt {
                word: "audit".to_string(),
                canonical_path: canonical_ext.clone(),
                current_trust: TrustState::Untrusted,
            })
        );
        assert_eq!(model.extensions[0].trust, TrustState::Untrusted);
        assert!(!marker.exists(), "Prompting must not execute extension");
        assert!(!sisho_log.exists(), "Prompting must not invoke sisho yet");

        // 3. Confirm Allowed (y): invokes `sisho --allow <canonical_path>`, queries extension
        //    and passes per-extension SISHO_CONFIG_DIR and SISHO_DATA_DIR
        model.apply_pending_trust(TrustState::Allowed);
        assert!(model.pending_trust.is_none());
        assert_eq!(model.extensions[0].trust, TrustState::Allowed);

        for _ in 0..40 {
            model.poll_background();
            if model.extensions[0].board_pane.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        assert!(marker.exists(), "Allowed extension should have been queried");
        assert!(model.extensions[0].info.is_some());
        assert!(model.extensions[0].board_pane.is_some());
        assert!(model.has_extension_panes());

        let envs = fs::read_to_string(&env_log).unwrap();
        let env_lines: Vec<&str> = envs.lines().collect();
        assert_eq!(env_lines.len(), 2, "{envs}");
        assert!(
            env_lines[0].ends_with("/sisho/audit"),
            "expected SISHO_CONFIG_DIR ending in /sisho/audit, got {:?}",
            env_lines[0]
        );
        assert!(
            env_lines[1].ends_with("/sisho/audit"),
            "expected SISHO_DATA_DIR ending in /sisho/audit, got {:?}",
            env_lines[1]
        );

        // 4. Confirm Denied (d): invokes `sisho --deny <canonical_path>`, clears pane, never executes
        let _ = fs::remove_file(&marker);
        model.request_trust_prompt();
        model.apply_pending_trust(TrustState::Denied);
        assert_eq!(model.extensions[0].trust, TrustState::Denied);
        assert!(model.extensions[0].info.is_none());
        assert!(model.extensions[0].board_pane.is_none());
        assert!(!model.has_extension_panes());

        model.query_allowed_extensions();
        std::thread::sleep(Duration::from_millis(50));
        model.poll_background();
        assert!(
            !marker.exists(),
            "Denied extension must never be executed"
        );

        // 5. Confirm Untrusted (u): invokes `sisho --untrust <canonical_path>`, transitions to Untrusted
        model.request_trust_prompt();
        model.apply_pending_trust(TrustState::Untrusted);
        assert_eq!(model.extensions[0].trust, TrustState::Untrusted);
        assert!(!marker.exists(), "Untrusted extension must never be executed");

        let sisho_calls = fs::read_to_string(&sisho_log).unwrap();
        let call_lines: Vec<&str> = sisho_calls.lines().collect();
        assert_eq!(
            call_lines,
            vec![
                format!("--allow {}", canonical_ext.display()),
                format!("--deny {}", canonical_ext.display()),
                format!("--untrust {}", canonical_ext.display()),
            ]
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }
}

#[test]
fn board_config_hidden_extension_panes_hides_pane_without_affecting_trust() {
    let mut model = create_test_model();
    model.extensions[0].trust = TrustState::Allowed;
    model.extensions[0].board_pane = Some(BoardPane::new("Desk Pane"));
    assert!(model.has_extension_panes());

    model
        .board_config
        .hidden_extension_panes
        .insert("desk".to_string());
    assert!(!model.has_extension_panes());
    assert!(model.active_extension_panes().is_empty());
    assert_eq!(model.extensions[0].trust, TrustState::Allowed);
}

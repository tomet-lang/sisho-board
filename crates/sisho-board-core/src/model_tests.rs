use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use sisho_board_health::health::{HealthReport, PillarMetric};
use sisho_board_health::links::{BrokenLink, Position, Report};
use sisho_board_run::{Outcome, Status};
use sisho_board_sdk::{BoardGauge, BoardItem, BoardPane, BoardStatus};
use sisho_sdk::{Capabilities, ExtensionInfo};

use crate::config::BoardConfig;
use crate::model::{
    ExtensionItem, FocusedPane, Model, PendingTrustPrompt, PreviewSource, StepSlot, WorkerMessage,
};
use crate::trust::TrustState;

fn sample_pillar(key: &str, name: &str, percent: u8, detail: &str) -> PillarMetric {
    PillarMetric {
        key: key.to_string(),
        name: name.to_string(),
        passed: percent as usize,
        total: 100,
        percent,
        status: sisho_board_health::status_for_percent(percent),
        value_summary: format!("{percent}%"),
        detail: detail.to_string(),
    }
}

fn sample_health_report() -> HealthReport {
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
        is_launcher_open: false,
        launcher_selected: 0,
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
            crate::unique_test_suffix()
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

fn make_info(word: &str, about: &str, tui: bool) -> ExtensionInfo {
    ExtensionInfo::new(word, "0.1.0", about)
        .with_capabilities(Capabilities::new().with_tui(tui))
}

#[test]
fn launcher_items_filters_by_active_word_trust_and_tui_capability() {
    let mut model = create_test_model();

    let mut board_ext = ExtensionItem::new("board", PathBuf::from("/bin/sisho-board"), true, None);
    board_ext.trust = TrustState::Allowed;
    board_ext.info = Some(make_info("board", "Board itself", true));

    let mut inactive_tui =
        ExtensionItem::new("shadowed", PathBuf::from("/alt/sisho-shadowed"), false, None);
    inactive_tui.trust = TrustState::Allowed;
    inactive_tui.info = Some(make_info("shadowed", "Shadowed TUI", true));

    let mut denied_ext = ExtensionItem::new("denied", PathBuf::from("/bin/sisho-denied"), true, None);
    denied_ext.trust = TrustState::Denied;

    let mut allowed_cli_only =
        ExtensionItem::new("audit", PathBuf::from("/bin/sisho-audit"), true, None);
    allowed_cli_only.trust = TrustState::Allowed;
    allowed_cli_only.info = Some(make_info("audit", "CLI audit", false));

    let mut allowed_unqueried =
        ExtensionItem::new("pending", PathBuf::from("/bin/sisho-pending"), true, None);
    allowed_unqueried.trust = TrustState::Allowed;
    allowed_unqueried.is_querying = true;

    let mut allowed_no_info =
        ExtensionItem::new("legacy", PathBuf::from("/bin/sisho-legacy"), true, None);
    allowed_no_info.trust = TrustState::Allowed;

    let mut allowed_tui = ExtensionItem::new("desk", PathBuf::from("/bin/sisho-desk"), true, None);
    allowed_tui.trust = TrustState::Allowed;
    allowed_tui.info = Some(make_info("desk", "Interactive desk", true));

    let untrusted_ext =
        ExtensionItem::new("history", PathBuf::from("/bin/sisho-history"), true, None);

    model.extensions = vec![
        board_ext,
        inactive_tui,
        denied_ext,
        allowed_cli_only,
        allowed_unqueried,
        allowed_no_info,
        allowed_tui,
        untrusted_ext,
    ];

    let items = model.launcher_items();
    let summary: Vec<(usize, &str, bool)> = items
        .iter()
        .map(|&(idx, ext)| (idx, ext.word.as_str(), ext.is_tui_launchable()))
        .collect();
    assert_eq!(summary, vec![(6, "desk", true), (7, "history", false)]);
}

#[test]
fn launcher_open_close_selection_and_clamping() {
    let mut model = create_test_model();

    let mut desk = ExtensionItem::new("desk", PathBuf::from("/bin/sisho-desk"), true, None);
    desk.trust = TrustState::Allowed;
    desk.info = Some(make_info("desk", "Desk", true));

    let history = ExtensionItem::new("history", PathBuf::from("/bin/sisho-history"), true, None);
    model.extensions = vec![desk, history];

    // Opening launcher clears transient prompts and clamps stale selection.
    model.confirm_fix = true;
    model.pending_trust = Some(PendingTrustPrompt {
        word: "desk".to_string(),
        canonical_path: PathBuf::from("/bin/sisho-desk"),
        current_trust: TrustState::Allowed,
    });
    model.launcher_selected = 99;
    model.open_launcher();

    assert!(model.is_launcher_open);
    assert!(!model.confirm_fix);
    assert!(model.pending_trust.is_none());
    assert_eq!(model.launcher_selected, 1);
    assert_eq!(
        model.selected_launcher_extension().map(|e| e.word.as_str()),
        Some("history")
    );

    // Up/down bounds
    model.move_launcher_selection_down();
    assert_eq!(model.launcher_selected, 1);
    model.move_launcher_selection_up();
    assert_eq!(model.launcher_selected, 0);
    assert_eq!(
        model.selected_launcher_extension().map(|e| e.word.as_str()),
        Some("desk")
    );
    model.move_launcher_selection_up();
    assert_eq!(model.launcher_selected, 0);

    model.close_launcher();
    assert!(!model.is_launcher_open);

    // Empty launcher items
    model.extensions.clear();
    model.launcher_selected = 5;
    model.open_launcher();
    assert_eq!(model.launcher_selected, 0);
    assert_eq!(model.selected_launcher_extension(), None);
    model.move_launcher_selection_down();
    assert_eq!(model.launcher_selected, 0);
}

#[test]
fn launcher_trust_prompt_and_queried_updates_keep_selection_valid() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = std::env::temp_dir().join(format!(
            "sisho-board-launcher-trust-{}",
            crate::unique_test_suffix()
        ));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let mock_sisho = temp_dir.join("sisho");
        fs::write(&mock_sisho, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&mock_sisho, fs::Permissions::from_mode(0o755)).unwrap();

        let mut model = create_test_model();
        model.sisho_program = mock_sisho;

        let mut denied = ExtensionItem::new("denied", PathBuf::from("/bin/sisho-denied"), true, None);
        denied.trust = TrustState::Denied;

        let mut desk = ExtensionItem::new("desk", PathBuf::from("/bin/sisho-desk"), true, None);
        desk.trust = TrustState::Allowed;
        desk.info = Some(make_info("desk", "Desk", true));

        let history = ExtensionItem::new("history", PathBuf::from("/bin/sisho-history"), true, None);

        // index 0 is filtered out; launcher has [(1, desk), (2, history)]
        model.extensions = vec![denied, desk, history];
        model.open_launcher();
        model.move_launcher_selection_down();
        assert_eq!(
            model.selected_launcher_extension().map(|e| e.word.as_str()),
            Some("history")
        );

        // Requesting trust prompt from launcher targets `history`, not `denied` or `desk`.
        model.request_launcher_trust_prompt();
        assert_eq!(
            model.pending_trust.as_ref().map(|p| p.word.as_str()),
            Some("history")
        );

        // Denying `history` removes it from launcher_items and clamps launcher_selected back to 0.
        model.apply_pending_trust(TrustState::Denied);
        assert_eq!(model.launcher_selected, 0);
        assert_eq!(
            model.selected_launcher_extension().map(|e| e.word.as_str()),
            Some("desk")
        );

        // If `desk` is re-queried and loses TUI capability, launcher_selected clamps to 0 / None.
        model
            .sender
            .send(WorkerMessage::ExtensionQueried {
                word: "desk".to_string(),
                info: Some(make_info("desk", "CLI only now", false)),
                board_pane: None,
            })
            .unwrap();
        model.poll_background();
        assert!(model.launcher_items().is_empty());
        assert_eq!(model.launcher_selected, 0);
        assert_eq!(model.selected_launcher_extension(), None);

        let _ = fs::remove_dir_all(&temp_dir);
    }
}

#[test]
fn launch_target_arg_uses_selected_broken_link_only_when_broken_links_focused() {
    let mut model = create_test_model();
    model.dir = PathBuf::from("/vault");
    model.links_report = Some(Report {
        files_scanned: 2,
        links_checked: 2,
        broken: vec![
            BrokenLink {
                source: PathBuf::from("docs/first.tmt"),
                target: "missing.tmt".to_string(),
                at: Some(Position { line: 3, column: 1 }),
            },
            BrokenLink {
                source: PathBuf::from("docs/second.tmt"),
                target: "gone.tmt".to_string(),
                at: Some(Position { line: 7, column: 2 }),
            },
        ],
        unreadable: Vec::new(),
    });

    // Not focused on BrokenLinks -> returns root dir
    model.focused_pane = FocusedPane::Checks;
    assert_eq!(model.launch_target_arg(), PathBuf::from("/vault"));

    // Focused on BrokenLinks -> returns joined source path for selected item
    model.focused_pane = FocusedPane::BrokenLinks;
    assert_eq!(
        model.launch_target_arg(),
        PathBuf::from("/vault/docs/first.tmt")
    );

    model.move_link_selection_down();
    assert_eq!(
        model.launch_target_arg(),
        PathBuf::from("/vault/docs/second.tmt")
    );

    // Empty broken links while focused -> falls back to root dir
    model.links_report = Some(Report::default());
    model.selected_broken_link = 0;
    assert_eq!(model.launch_target_arg(), PathBuf::from("/vault"));
}

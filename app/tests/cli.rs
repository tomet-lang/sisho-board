//! Integration tests for the `sisho-board` CLI binary.

use std::fs;
use std::process::Command;

use sisho_board_core::health::HealthReport;
use sisho_sdk::ExtensionInfo;

#[test]
fn sisho_extension_info_prints_valid_metadata_and_exits_0() {
    let output = Command::new(env!("CARGO_BIN_EXE_sisho-board"))
        .arg("--sisho-extension-info")
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let info = ExtensionInfo::from_json_str(&stdout).unwrap();
    assert_eq!(info.word, "board");
    assert!(info.capabilities.tui);
    assert!(info.capabilities.interactive);
}

#[test]
fn non_terminal_invocation_exits_1_with_message() {
    let output = Command::new(env!("CARGO_BIN_EXE_sisho-board"))
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("standard input and output must be a terminal"),
        "{stderr}"
    );
}

#[test]
fn report_and_json_flags_run_non_interactively_and_exit_0() {
    let temp_dir = std::env::temp_dir().join(format!(
        "sisho-board-cli-report-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(temp_dir.join("tmtroot")).unwrap();

    fs::write(
        temp_dir.join(".writ.tmt"),
        "::kind(writ)\n@rule(layering){\n  guard: (twrit: layers)\n}\n",
    )
    .unwrap();
    fs::write(
        temp_dir.join("tmtroot/readme.tmt"),
        "::id(doc-readme)\n::kind(guide)\n=[ Hello ]\n",
    )
    .unwrap();

    // 1. --report
    let text_out = Command::new(env!("CARGO_BIN_EXE_sisho-board"))
        .arg("--report")
        .arg(&temp_dir)
        .output()
        .unwrap();
    assert!(text_out.status.success());
    let text = String::from_utf8(text_out.stdout).unwrap();
    assert!(text.contains("Vault Readiness & Health"), "{text}");
    assert!(text.contains("Overall Readiness"), "{text}");
    assert!(text.contains("Tomet Coverage:"), "{text}");
    assert!(text.contains("::id Declaration:"), "{text}");

    // 2. --json
    let json_out = Command::new(env!("CARGO_BIN_EXE_sisho-board"))
        .arg("--json")
        .arg(&temp_dir)
        .output()
        .unwrap();
    assert!(json_out.status.success());
    let json_str = String::from_utf8(json_out.stdout).unwrap();
    let parsed: HealthReport = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed.tomet_coverage.percent, 100);
    assert_eq!(parsed.id_coverage.percent, 100);
    assert_eq!(parsed.kind_coverage.percent, 100);
    assert_eq!(parsed.governance.percent, 100);

    let _ = fs::remove_dir_all(&temp_dir);
}

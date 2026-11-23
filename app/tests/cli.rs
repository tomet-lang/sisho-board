//! Integration tests for the `sisho-board` CLI binary.

use std::process::Command;

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

// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

mod common;
use common::*;
use std::fs;
use std::io::Write;
use std::process::Command;
use tempfile::TempDir;

#[test]
fn import_text() {
    vprintln("e2e_import_text: importing from file");
    let dir = TempDir::new().expect("tempdir");
    let import_path = dir.path().join("import.txt");
    fs::write(&import_path, "offline-file-test\n").expect("write import");
    let config_path = write_temp_config(&dir, "");
    let output = run_dsc(&["import", import_path.to_str().unwrap()], &config_path);
    assert!(output.status.success(), "import failed");
    let raw = fs::read_to_string(config_path).expect("read config");
    assert!(raw.contains("offline-file-test"));
}

#[test]
fn import_stdin() {
    vprintln("e2e_import_stdin: importing from stdin");
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(&dir, "");
    let mut child = Command::new(env!("CARGO_BIN_EXE_dsc"))
        .arg("-c")
        .arg(&config_path)
        .arg("import")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()
        .expect("spawn import");
    {
        let stdin = child.stdin.as_mut().expect("stdin");
        stdin
            .write_all(b"offline-stdin-test\n")
            .expect("write stdin");
    }
    let status = child.wait().expect("wait");
    assert!(status.success(), "import stdin failed");
    let raw = fs::read_to_string(config_path).expect("read config");
    assert!(raw.contains("offline-stdin-test"));
}

#[test]
fn import_title_lookup_diagnostic_uses_stderr() {
    let dir = TempDir::new().expect("tempdir");
    let import_path = dir.path().join("import.txt");
    fs::write(&import_path, "not a URL\n").expect("write import");
    let config_path = write_temp_config(&dir, "");

    let output = run_dsc(&["import", import_path.to_str().unwrap()], &config_path);

    assert!(output.status.success(), "import failed");
    assert!(
        output.stdout.is_empty(),
        "import diagnostic leaked to stdout"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("Failed to query site title"));
}

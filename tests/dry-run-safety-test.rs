// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

fn run_with_missing_config(args: &[&str], config_path: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dsc"))
        .args(args)
        .env("DSC_CONFIG", config_path)
        .env_remove("DSC_CONFIG_HOME")
        .output()
        .expect("run dsc")
}

#[test]
fn dry_run_unsafe_commands_short_circuit_before_config_resolution() {
    let dir = TempDir::new().expect("tempdir");
    let missing_config = dir.path().join("must-not-be-read.toml");
    let missing_input = dir.path().join("must-not-be-read.png");
    let missing_input = missing_input.to_str().expect("UTF-8 temporary path");

    assert!(
        !missing_config.exists(),
        "the test requires DSC_CONFIG to name a nonexistent file"
    );

    let commands = [
        ("update", vec!["--dry-run", "update", "example"]),
        (
            "emoji push",
            vec!["--dry-run", "emoji", "push", "example", missing_input],
        ),
        (
            "backup create",
            vec!["--dry-run", "backup", "create", "example"],
        ),
        (
            "theme update",
            vec!["--dry-run", "theme", "update", "example", "1"],
        ),
        (
            "upload",
            vec!["--dry-run", "upload", "example", missing_input],
        ),
        ("config check", vec!["--dry-run", "config", "check"]),
        ("doctor", vec!["--dry-run", "doctor"]),
    ];

    for (name, args) in commands {
        let output = run_with_missing_config(&args, &missing_config);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{stdout}{stderr}");

        assert!(
            !output.status.success(),
            "{name} must refuse before resolving $DSC_CONFIG={}; \
             stdout: {stdout}\nstderr: {stderr}",
            missing_config.display(),
        );
        assert!(
            combined.contains("[dry-run]"),
            "{name} must print the [dry-run] refusal marker before configuration \
             is resolved; stdout: {stdout}\nstderr: {stderr}",
        );
        assert!(
            !missing_config.exists(),
            "{name} must not create or replace the missing DSC_CONFIG path",
        );
    }
}

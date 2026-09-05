// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use std::fs;
use std::process::Command;
use tempfile::TempDir;

#[test]
fn generation_commands_do_not_resolve_config() {
    let dir = TempDir::new().expect("tempdir");
    let missing_config = dir.path().join("missing.toml");
    let malformed_config = dir.path().join("malformed.toml");
    fs::write(&malformed_config, "not valid = [").expect("write malformed config");

    for (case, config) in [
        ("missing", missing_config.as_path()),
        ("malformed", malformed_config.as_path()),
    ] {
        let completions_dir = dir.path().join(format!("completions-{case}"));
        let output = Command::new(env!("CARGO_BIN_EXE_dsc"))
            .arg("--config")
            .arg(config)
            .args(["completions", "bash", "--dir"])
            .arg(&completions_dir)
            .output()
            .expect("run dsc completions");
        assert!(
            output.status.success(),
            "completions must ignore {case} config: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(completions_dir.join("dsc").is_file());

        let man_dir = dir.path().join(format!("man-{case}"));
        let output = Command::new(env!("CARGO_BIN_EXE_dsc"))
            .arg("--config")
            .arg(config)
            .args(["man", "--dir"])
            .arg(&man_dir)
            .output()
            .expect("run dsc man");
        assert!(
            output.status.success(),
            "man must ignore {case} config: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(man_dir.join("dsc.1").is_file());
    }
}

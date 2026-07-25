// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

mod common;
use common::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn completions_generate() {
    vprintln("e2e_completions: generate completions");
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(&dir, "");
    let out_dir = dir.path().join("completions");
    let out_dir_str = out_dir.to_str().expect("out dir");

    let output = run_dsc(&["completions", "bash", "--dir", out_dir_str], &config_path);
    assert!(output.status.success(), "bash completions failed");
    assert!(out_dir.join("dsc").exists(), "missing dsc");

    let output = run_dsc(&["completions", "zsh", "--dir", out_dir_str], &config_path);
    assert!(output.status.success(), "zsh completions failed");
    assert!(out_dir.join("_dsc").exists(), "missing _dsc");

    let output = run_dsc(&["completions", "fish", "--dir", out_dir_str], &config_path);
    assert!(output.status.success(), "fish completions failed");
    assert!(out_dir.join("dsc.fish").exists(), "missing dsc.fish");

    // `powershell` must be accepted verbatim (the value enum derives to
    // `power-shell`, so the canonical spelling the docs use is a named alias).
    let output = run_dsc(
        &["completions", "powershell", "--dir", out_dir_str],
        &config_path,
    );
    assert!(output.status.success(), "powershell completions failed");
    assert!(out_dir.join("dsc.ps1").exists(), "missing dsc.ps1");

    let output = run_dsc(
        &[
            "completions",
            "install",
            "--shell",
            "zsh",
            "--dir",
            out_dir_str,
        ],
        &config_path,
    );
    assert!(output.status.success(), "zsh install completions failed");
    assert!(out_dir.join("_dsc").exists(), "missing installed _dsc");

    let entries: Vec<_> = fs::read_dir(&out_dir)
        .expect("read completions dir")
        .filter_map(|entry| entry.ok())
        .collect();
    assert!(entries.len() >= 3, "unexpected completions count");

    // Completions are generated from the clap CLI, so newly-added commands
    // appear automatically - guard that a representative sample is present
    // (catches a command silently dropping out of the surface).
    let zsh = fs::read_to_string(out_dir.join("_dsc")).expect("read _dsc");
    for cmd in ["setup-s3", "sar", "audit", "version", "title"] {
        assert!(zsh.contains(cmd), "zsh completions missing `{cmd}`");
    }

    // The zsh post-processing rewrites every `<discourse>` positional to the
    // dynamic `_dsc_discourse_names` completer. That injection is a fragile
    // string match against clap_complete's output format, so assert it took:
    // the helper is present, and no `:discourse` arg was left on `:_default`.
    assert!(
        zsh.contains("_dsc_discourse_names"),
        "dynamic discourse-name completion was not injected"
    );
    let mut discourse_args = 0;
    let mut idx = 0;
    while let Some(p) = zsh[idx..].find("':discourse") {
        let start = idx + p;
        let rest = &zsh[start..];
        // Whichever completer closes this arg must be the dynamic one.
        let dynamic = rest.find(":_dsc_discourse_names'");
        let default = rest.find(":_default'");
        match (dynamic, default) {
            (Some(dy), Some(de)) => assert!(
                dy < de,
                "a `:discourse` arg still falls through to :_default (injection broke)"
            ),
            (None, Some(_)) => {
                panic!("a `:discourse` arg still uses :_default (injection broke)")
            }
            _ => {}
        }
        discourse_args += 1;
        idx = start + "':discourse".len();
    }
    assert!(
        discourse_args > 5,
        "expected many `:discourse` positionals, found {discourse_args}"
    );
}

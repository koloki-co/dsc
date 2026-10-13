// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! R72: hostile-environment coverage for config discovery.
//!
//! `spec/commands/config-path-resolution.md`'s "Explicit-selector semantics
//! (safety)" section states the contract in prose: an explicit selector
//! (`--config`/`$DSC_CONFIG`) naming a missing file must error, never fall
//! through to a lower-precedence config. `src/config.rs`'s own unit tests
//! (`flag_wins_over_env_and_discovery`, `missing_dsc_config_env_path_errors`,
//! ...) already prove this at the pure-function level, calling
//! `resolve_config_source_with_env` directly with an injected environment
//! map - a white-box check of the resolution logic in isolation.
//!
//! This file proves the same contract black-box, through the compiled
//! `dsc` binary, with every discovery location populated by a genuinely
//! misleading file an inattentive implementation could fall back to: a
//! valid-but-wrong config at `$HOME/.config/dsc/dsc.toml`, a second
//! valid-but-wrong config at `./dsc.toml` in the process's working
//! directory, each naming a discourse whose baseurl is a closed port
//! (`http://127.0.0.1:1`, the existing house convention elsewhere in
//! `tests/` for "a request here must never happen"). If `dsc` ever actually
//! resolved to the wrong file, a live command would either report the wrong
//! baseurl or fail on connection-refused instead of succeeding against the
//! real mock. Each test also snapshots both hostile directories before and
//! after, asserting `dsc` wrote nothing outside what it was explicitly
//! asked to write.

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

/// A forum baseurl that must never actually be contacted - closed/reserved
/// port, same convention already used elsewhere in `tests/` for proving a
/// command never got far enough to attempt a connection.
const UNREACHABLE_BASEURL: &str = "http://127.0.0.1:1";

/// A config naming one discourse called `winner`, so every file in a test's
/// hostile sandbox can share the same discourse *name* and only the baseurl
/// tells the real config apart from a decoy.
fn discourse_toml(baseurl: &str) -> String {
    format!(
        "[[discourse]]\nname = \"winner\"\nbaseurl = \"{baseurl}\"\napikey = \"k\"\napi_username = \"tester\"\n"
    )
}

/// A minimal mock that always answers `/about.json` with 200 OK, enough for
/// `dsc config check --skip-ssh` to report `api.ok: true`.
fn start_about_mock() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock listener");
    let addr = listener.local_addr().expect("mock addr");
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            handle_about(stream);
        }
    });
    format!("http://{addr}")
}

fn handle_about(mut stream: TcpStream) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone mock stream"));
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() || line == "\r\n" || line == "\n" {
            break;
        }
    }
    let body = r#"{"about":{"version":"9.9.9"}}"#;
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
}

/// Recursively snapshot every regular file's relative path and content
/// under `dir`, so a test can assert a read-only command wrote nothing at
/// all - not a new file, not a changed byte - anywhere inside it.
fn snapshot_dir(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).expect("read dir") {
            let entry = entry.expect("dir entry");
            let path = entry.path();
            if path.is_dir() {
                walk(base, &path, out);
            } else {
                let relative = path
                    .strip_prefix(base)
                    .expect("relative path")
                    .to_path_buf();
                out.insert(relative, fs::read(&path).expect("read file"));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

/// Run `dsc` with no `-c` flag - so discovery and `$DSC_CONFIG` are
/// genuinely exercised - inside a hostile, fully-isolated environment:
/// a controlled working directory, a controlled `$HOME`, and every
/// discovery-relevant env var explicitly removed unless the caller
/// supplies it, so nothing leaks in from the environment the test
/// binary itself happens to run under.
fn run_hostile(args: &[&str], cwd: &Path, home: &Path, extra_env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_dsc"));
    cmd.current_dir(cwd)
        .env("HOME", home)
        .env_remove("DSC_CONFIG")
        .env_remove("DSC_CONFIG_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_CONFIG_DIRS")
        .args(args);
    for (key, value) in extra_env {
        cmd.env(key, value);
    }
    cmd.output().expect("run dsc")
}

/// Build the two misleading discoverable locations - `$HOME/.config/dsc/`
/// and a working directory - inside a fresh sandbox, and return their
/// paths. Callers populate `dsc.toml` in each as the scenario requires.
fn hostile_sandbox() -> (TempDir, PathBuf, PathBuf) {
    let sandbox = TempDir::new().expect("tempdir");
    let home = sandbox.path().join("home");
    let cwd = sandbox.path().join("cwd");
    fs::create_dir_all(home.join(".config").join("dsc")).expect("mkdir home config");
    fs::create_dir_all(&cwd).expect("mkdir cwd");
    (sandbox, home, cwd)
}

fn home_config_path(home: &Path) -> PathBuf {
    home.join(".config").join("dsc").join("dsc.toml")
}

#[test]
fn explicit_dsc_config_wins_over_misleading_discoverable_files_and_contacts_only_that_forum() {
    let mock_url = start_about_mock();
    let (sandbox, home, cwd) = hostile_sandbox();

    // Both discoverable locations are perfectly valid TOML naming the same
    // discourse name as the real config - only the baseurl can tell them
    // apart - so a correct implementation must never even read them.
    let decoy = discourse_toml(UNREACHABLE_BASEURL);
    fs::write(home_config_path(&home), &decoy).expect("write home decoy");
    fs::write(cwd.join("dsc.toml"), &decoy).expect("write cwd decoy");

    let real_config = sandbox.path().join("real.toml");
    fs::write(&real_config, discourse_toml(&mock_url)).expect("write real config");

    let before_home = snapshot_dir(&home);
    let before_cwd = snapshot_dir(&cwd);

    let dsc_config = real_config.to_str().expect("utf-8 path").to_string();
    let output = run_hostile(
        &["config", "check", "--skip-ssh", "--format", "json"],
        &cwd,
        &home,
        &[("DSC_CONFIG", &dsc_config)],
    );

    assert!(
        output.status.success(),
        "expected the real mock's response, not a decoy's closed port: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let reports: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse report");
    let reports = reports.as_array().expect("report array");
    assert_eq!(reports.len(), 1, "exactly one discourse must be checked");
    assert_eq!(
        reports[0]["baseurl"], mock_url,
        "the checked forum must be the real config's, not a decoy's: {stdout}"
    );
    assert_eq!(
        reports[0]["api"]["ok"], true,
        "the real mock must have actually been contacted: {stdout}"
    );
    assert!(
        !stdout.contains('\u{1b}'),
        "piped, non-terminal output must carry no raw ANSI escapes: {stdout:?}"
    );

    assert_eq!(
        snapshot_dir(&home),
        before_home,
        "a read-only `config check` must not write inside $HOME, including the decoy"
    );
    assert_eq!(
        snapshot_dir(&cwd),
        before_cwd,
        "a read-only `config check` must not write inside the working directory"
    );
}

#[test]
fn missing_explicit_dsc_config_errors_without_falling_through_to_valid_discoverable_files() {
    let (sandbox, home, cwd) = hostile_sandbox();

    // Both discoverable locations are valid and would check out cleanly if
    // dsc silently fell through to them - proving the refusal is real, not
    // just a side effect of there being nothing usable to fall through to.
    let decoy = discourse_toml(UNREACHABLE_BASEURL);
    fs::write(home_config_path(&home), &decoy).expect("write home decoy");
    fs::write(cwd.join("dsc.toml"), &decoy).expect("write cwd decoy");

    let before_home = snapshot_dir(&home);
    let before_cwd = snapshot_dir(&cwd);

    let missing = sandbox.path().join("does-not-exist.toml");
    let missing = missing.to_str().expect("utf-8 path").to_string();
    let output = run_hostile(
        &["config", "check", "--skip-ssh"],
        &cwd,
        &home,
        &[("DSC_CONFIG", &missing)],
    );

    assert!(
        !output.status.success(),
        "a missing $DSC_CONFIG target must refuse, not silently use a decoy"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("$DSC_CONFIG"),
        "error must name the explicit selector that failed: {stderr}"
    );
    assert!(
        output.stdout.is_empty(),
        "nothing should ever be printed once the explicit selector fails - \
         in particular, no 'Checking 1 discourse(s)' signpost or report, \
         which would mean a decoy was used: {}",
        String::from_utf8_lossy(&output.stdout)
    );

    assert_eq!(
        snapshot_dir(&home),
        before_home,
        "a refused command must not write inside $HOME"
    );
    assert_eq!(
        snapshot_dir(&cwd),
        before_cwd,
        "a refused command must not write inside the working directory"
    );
}

#[test]
fn malformed_file_at_the_winning_discovered_path_errors_without_trying_the_next_candidate() {
    let (_sandbox, home, cwd) = hostile_sandbox();

    // ./dsc.toml (step 1, higher precedence) exists but is garbage.
    // $HOME/.config/dsc/dsc.toml (step 2) is perfectly valid - it must not
    // be used once the higher-precedence candidate has already committed
    // to existing, even though it fails to parse.
    fs::write(cwd.join("dsc.toml"), "this is not [[[ valid toml").expect("write malformed cwd");
    let decoy = discourse_toml(UNREACHABLE_BASEURL);
    fs::write(home_config_path(&home), &decoy).expect("write home decoy");

    let before_home = snapshot_dir(&home);
    let before_cwd = snapshot_dir(&cwd);

    // No $DSC_CONFIG: discovery alone must pick ./dsc.toml and stop there.
    let output = run_hostile(&["config", "check", "--skip-ssh"], &cwd, &home, &[]);

    assert!(
        !output.status.success(),
        "a malformed higher-precedence candidate must refuse, not silently \
         fall through to the valid one behind it"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("parsing config"),
        "error must report a parse failure, not an unrelated one: {stderr}"
    );
    assert!(
        output.stdout.is_empty(),
        "nothing should be printed once the winning candidate fails to \
         parse - in particular, no report naming the fallback decoy: {}",
        String::from_utf8_lossy(&output.stdout)
    );

    assert_eq!(
        snapshot_dir(&home),
        before_home,
        "a refused command must not write inside $HOME"
    );
    assert_eq!(
        snapshot_dir(&cwd),
        before_cwd,
        "a refused command must not write inside the working directory, \
         including rewriting the malformed file itself"
    );
}

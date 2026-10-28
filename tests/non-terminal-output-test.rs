// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! Non-terminal output guarantee (R72): when stdout/stderr are pipes, `dsc`
//! must not write raw ANSI/terminal escape sequences unless the operator
//! explicitly forced them. Every surface that can emit an escape is run
//! through a real pipe here, with the inherited terminal hints a user's shell
//! would carry, and each suppression test has a forced-on positive control so
//! the `\x1b` check is proven able to fail.

mod common;
use common::*;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

const ESC: char = '\u{1b}';

/// Environment that influences colour/escape decisions. Removed from every
/// run so the developer's own shell cannot make a test pass or fail.
const ESCAPE_ENV: &[&str] = &[
    "DSC_COLOR",
    "NO_COLOR",
    "CLICOLOR",
    "CLICOLOR_FORCE",
    "DSC_EMOJI_INLINE_PROTOCOL",
    "TERM_PROGRAM",
    "KITTY_WINDOW_ID",
    "KITTY_SESSION_ID",
];

/// Run the compiled binary with piped stdout/stderr (as `Command::output`
/// always does), a scrubbed escape-relevant environment, and `envs` on top.
fn run_piped(args: &[&str], config: &Path, home: &Path, envs: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_dsc"));
    cmd.arg("-c").arg(config).args(args);
    for key in ESCAPE_ENV {
        cmd.env_remove(key);
    }
    cmd.env("HOME", home)
        .env("XDG_STATE_HOME", home.join("state"))
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("TERM", "xterm-256color");
    for (key, value) in envs {
        cmd.env(key, value);
    }
    cmd.output().expect("run dsc")
}

fn assert_no_escapes(output: &Output, context: &str) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stdout.contains(ESC),
        "{context}: piped stdout must carry no escapes: {stdout:?}"
    );
    assert!(
        !stderr.contains(ESC),
        "{context}: piped stderr must carry no escapes: {stderr:?}"
    );
}

// ---------------------------------------------------------------------------
// `dsc update` per-forum labels (the only coloured label surface)
// ---------------------------------------------------------------------------

/// Config for one SSH-backed forum with a truecolour `update_colour`, plus a
/// `PATH` directory whose `ssh` refuses every connection. `dsc update` prints
/// its coloured `==> Updating` header and first stage line before the first
/// SSH call, then fails fast on the refusal - no network, no real host.
#[cfg(unix)]
fn update_sandbox(dir: &TempDir) -> (std::path::PathBuf, String) {
    use std::os::unix::fs::PermissionsExt;

    let config = write_temp_config(
        dir,
        "[[discourse]]\nname = \"mock\"\nbaseurl = \"http://127.0.0.1:9\"\napikey = \"key\"\napi_username = \"system\"\nssh_host = \"mock.invalid\"\nupdate_colour = \"#3f8f77\"\n",
    );
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).expect("create stub bin dir");
    let ssh = bin.join("ssh");
    std::fs::write(
        &ssh,
        "#!/bin/sh\necho 'ssh: connect to host mock.invalid: Connection refused' >&2\nexit 255\n",
    )
    .expect("write ssh stub");
    std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o755)).expect("chmod ssh stub");
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    (config, path)
}

#[cfg(unix)]
#[test]
fn update_labels_are_plain_when_piped() {
    let dir = TempDir::new().expect("tempdir");
    let (config, path) = update_sandbox(&dir);

    let output = run_piped(
        &["update", "mock", "-y"],
        &config,
        dir.path(),
        &[("PATH", &path)],
    );

    assert!(
        !output.status.success(),
        "the refusing ssh stub must fail the update"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("==> Updating mock (mock.invalid)"),
        "the label must still be printed, just uncoloured: {stdout}"
    );
    assert!(
        stdout.contains("[mock] Checking for an in-progress rebuild"),
        "stage lines must carry the plain label: {stdout}"
    );
    assert_no_escapes(&output, "update (auto colour, piped)");
}

#[cfg(unix)]
#[test]
fn update_labels_colour_only_when_forced_and_no_color_wins() {
    let dir = TempDir::new().expect("tempdir");
    let (config, path) = update_sandbox(&dir);

    // Positive control: forcing colour through a pipe must emit the
    // configured truecolour, proving the no-escape assertions can fail.
    let forced = run_piped(
        &["update", "mock", "-y"],
        &config,
        dir.path(),
        &[("PATH", &path), ("DSC_COLOR", "always")],
    );
    let stdout = String::from_utf8_lossy(&forced.stdout);
    assert!(
        stdout.contains("\u{1b}[1;38;2;63;143;119mmock\u{1b}[0m"),
        "DSC_COLOR=always must colour the label even when piped: {stdout:?}"
    );

    // NO_COLOR overrides an explicit DSC_COLOR=always.
    let suppressed = run_piped(
        &["update", "mock", "-y"],
        &config,
        dir.path(),
        &[("PATH", &path), ("DSC_COLOR", "always"), ("NO_COLOR", "1")],
    );
    assert_no_escapes(&suppressed, "update (NO_COLOR + DSC_COLOR=always)");
}

// ---------------------------------------------------------------------------
// `dsc emoji list --inline` terminal image protocols
// ---------------------------------------------------------------------------

/// A 1x1 transparent PNG; the bytes are only base64-encoded, never decoded.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae,
    0x42, 0x60, 0x82,
];

/// Serve the emoji list (one emoji whose image lives on this same mock) and
/// the image itself, for as many connections as the run makes.
fn start_emoji_mock() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock listener");
    let base = format!("http://{}", listener.local_addr().expect("mock addr"));
    let list = format!(r#"[{{"name":"partytime","url":"{base}/uploads/partytime.png"}}]"#);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            serve_emoji_request(stream, &list);
        }
    });
    base
}

fn serve_emoji_request(mut stream: TcpStream, list: &str) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone mock stream"));
    let mut request_line = String::new();
    reader.read_line(&mut request_line).expect("request line");
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).expect("request header");
        if line == "\r\n" || line == "\n" || line.is_empty() {
            break;
        }
    }
    let (content_type, body): (&str, &[u8]) = if request_line.contains("/uploads/partytime.png") {
        ("image/png", PNG)
    } else if request_line.contains("/admin/customize/emojis.json") {
        ("application/json", list.as_bytes())
    } else {
        let _ = stream
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        return;
    };
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
}

fn emoji_config(dir: &TempDir) -> std::path::PathBuf {
    let url = start_emoji_mock();
    write_temp_config(
        dir,
        &format!(
            "[[discourse]]\nname = \"mock\"\nbaseurl = \"{url}\"\napikey = \"key\"\napi_username = \"system\"\n"
        ),
    )
}

#[test]
fn emoji_inline_autodetection_is_ignored_when_piped() {
    let dir = TempDir::new().expect("tempdir");
    let config = emoji_config(&dir);

    // Each hint a terminal leaves in the environment of everything it spawns,
    // including `dsc ... | grep` and `dsc ... > file`.
    for hint in [
        ("TERM_PROGRAM", "iTerm.app"),
        ("TERM_PROGRAM", "WezTerm"),
        ("KITTY_WINDOW_ID", "1"),
        ("TERM", "xterm-kitty"),
    ] {
        let output = run_piped(
            &["emoji", "list", "mock", "--inline"],
            &config,
            dir.path(),
            &[hint],
        );
        assert!(
            output.status.success(),
            "{hint:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("name\turl") && stdout.contains("partytime\t"),
            "{hint:?}: piped --inline must fall back to the plain table: {stdout:?}"
        );
        assert_no_escapes(&output, &format!("emoji list --inline with {hint:?}"));
    }
}

#[test]
fn emoji_inline_explicit_protocol_still_forces_escapes() {
    let dir = TempDir::new().expect("tempdir");
    let config = emoji_config(&dir);

    // Positive control and documented override: an explicit protocol is the
    // operator's choice and is honoured even into a pipe.
    for (protocol, prefix) in [("kitty", "\u{1b}_G"), ("iterm2", "\u{1b}]1337;")] {
        let output = run_piped(
            &["emoji", "list", "mock", "--inline"],
            &config,
            dir.path(),
            &[("DSC_EMOJI_INLINE_PROTOCOL", protocol)],
        );
        assert!(
            output.status.success(),
            "{protocol}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains(&format!("partytime {prefix}")),
            "DSC_EMOJI_INLINE_PROTOCOL={protocol} must emit its image escape: {stdout:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// clap help and usage errors
// ---------------------------------------------------------------------------

#[test]
fn help_and_usage_errors_are_plain_when_piped() {
    let dir = TempDir::new().expect("tempdir");
    let config = write_temp_config(&dir, "");

    let help = run_piped(&["--help"], &config, dir.path(), &[]);
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("Usage:"));
    assert_no_escapes(&help, "dsc --help");

    let sub_help = run_piped(&["setting", "--help"], &config, dir.path(), &[]);
    assert!(sub_help.status.success());
    assert_no_escapes(&sub_help, "dsc setting --help");

    let error = run_piped(&["no-such-command"], &config, dir.path(), &[]);
    assert!(!error.status.success());
    assert!(String::from_utf8_lossy(&error.stderr).contains("no-such-command"));
    assert_no_escapes(&error, "dsc no-such-command");
}

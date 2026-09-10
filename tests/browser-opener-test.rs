// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

#![cfg(unix)]

mod common;

use std::fs;
use std::io::Write;
use std::os::unix::{fs::PermissionsExt, process::CommandExt};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tempfile::TempDir;

const TIMEOUT: Duration = Duration::from_secs(5);

struct OpenerFixture {
    dir: TempDir,
    group: Option<i32>,
    reader: Option<JoinHandle<()>>,
}

impl OpenerFixture {
    fn new(slow: bool) -> Self {
        let dir = TempDir::new().unwrap();
        common::write_temp_config(
            &dir,
            r#"[[discourse]]
name = "one"
baseurl = "https://one.example"
tags = ["alpha"]
[[discourse]]
name = "two"
baseurl = "https://two.example"
tags = ["gamma"]
[[discourse]]
name = "three"
baseurl = "https://three.example"
tags = ["gamma"]
"#,
        );
        // The shell parses the whole brace group before signalling readiness.
        // Tests wait for that handshake before allowing the script to be deleted.
        let script = format!(
            "#!/bin/sh\n{{\nprintf 'opener stdout\\n'\nprintf 'opener stderr\\n' >&2\ninput=EOF\nread -r input || input=EOF\nprintf '%s\\n' \"$#\" \"$@\" \"$input\" > \"$DSC_OPENER_TEST_DIR/args-$$\"\n: > \"$DSC_OPENER_TEST_DIR/ready-$$\"\n{}\n}}\n",
            if slow { "exec sleep 60" } else { "exit 7" }
        );
        let opener = dir.path().join("opener.sh");
        fs::write(&opener, script).unwrap();
        fs::set_permissions(opener, fs::Permissions::from_mode(0o755)).unwrap();
        Self {
            dir,
            group: None,
            reader: None,
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_dsc"));
        cmd.arg("--config")
            .arg(self.dir.path().join("dsc.toml"))
            .args(args);
        cmd
    }

    fn start(&mut self, mut cmd: Command, missing: bool) -> Receiver<Output> {
        cmd.env("DSC_OPENER_TEST_DIR", self.dir.path())
            .env(
                "DSC_BROWSER_OPENER",
                self.dir
                    .path()
                    .join(if missing { "missing" } else { "opener.sh" }),
            )
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().unwrap();
        self.group = Some(child.id() as i32);
        let _ = child.stdin.take().unwrap().write_all(b"interactive\n");
        let (tx, rx) = mpsc::channel();
        self.reader = Some(thread::spawn(move || {
            let _ = tx.send(child.wait_with_output().unwrap());
        }));
        rx
    }

    fn records(&self, count: usize) -> Vec<String> {
        let mut records = Vec::new();
        wait_until(|| {
            records = fs::read_dir(self.dir.path())
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .filter_map(|name| {
                    name.strip_prefix("ready-").map(|pid| {
                        fs::read_to_string(self.dir.path().join(format!("args-{pid}"))).unwrap()
                    })
                })
                .collect();
            records.len() >= count
        });
        records.sort();
        records
    }
}

impl Drop for OpenerFixture {
    fn drop(&mut self) {
        if let Some(group) = self.group {
            // Only this fixture's process group, including a hung CLI and opener.
            unsafe { libc::kill(-group, libc::SIGKILL) };
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn wait_until(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    while !ready() {
        assert!(
            Instant::now() < deadline,
            "opener handshake/reaping timed out"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn list_open_launches_exact_selected_urls_without_waiting_or_inheriting_stdio() {
    for filtered in [false, true] {
        let mut fixture = OpenerFixture::new(true);
        let mut args = vec!["list", "--open", "-f", "urls"];
        if filtered {
            args.extend(["--tags", "gamma"]);
        }
        let rx = fixture.start(fixture.command(&args), false);
        // This bounds both process exit and pipe EOF, not just elapsed time
        // measured after a potentially endless Command::output().
        let output = rx.recv_timeout(TIMEOUT).expect("list --open timed out");
        assert!(output.status.success(), "{output:?}");
        let urls = if filtered {
            vec!["https://two.example", "https://three.example"]
        } else {
            vec![
                "https://one.example",
                "https://two.example",
                "https://three.example",
            ]
        };
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("{}\n", urls.join("\n"))
        );
        assert!(output.stderr.is_empty(), "{output:?}");
        let mut expected: Vec<_> = urls.iter().map(|url| format!("1\n{url}\nEOF\n")).collect();
        expected.sort();
        assert_eq!(fixture.records(urls.len()), expected);
    }
}

#[test]
fn missing_opener_is_reported_by_both_commands() {
    for args in [&["list", "--open"][..], &["open", "one"][..]] {
        let mut fixture = OpenerFixture::new(false);
        let rx = fixture.start(fixture.command(args), true);
        let output = rx.recv_timeout(TIMEOUT).expect("command timed out");
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("failed to launch browser opener")
        );
    }
}

#[test]
fn list_open_reports_launch_success_not_late_exit_failure() {
    let mut fixture = OpenerFixture::new(false);
    let rx = fixture.start(
        fixture.command(&["list", "--open", "--tags", "alpha", "-f", "urls"]),
        false,
    );
    let output = rx.recv_timeout(TIMEOUT).expect("list --open timed out");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"https://one.example\n");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(fixture.records(1), ["1\nhttps://one.example\nEOF\n"]);
}

#[test]
fn standalone_open_checks_nonzero_exit_and_inherits_stdio() {
    let mut fixture = OpenerFixture::new(false);
    let rx = fixture.start(fixture.command(&["open", "one"]), false);
    let output = rx.recv_timeout(TIMEOUT).expect("open timed out");
    assert!(!output.status.success());
    assert_eq!(output.stdout, b"opener stdout\n");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("opener stderr\n"));
    assert!(stderr.contains("browser opener exited with status"));
    assert!(stderr.contains('7'));
    assert_eq!(
        fixture.records(1),
        ["1\nhttps://one.example\ninteractive\n"]
    );
}

#[cfg(target_os = "linux")]
#[test]
fn detached_opener_is_reaped_in_a_living_library_host() {
    if let Ok(dir) = std::env::var("DSC_OPENER_TEST_DIR") {
        dsc::commands::common::open_url_detached("https://one.example").unwrap();
        let mut pid = String::new();
        wait_until(|| {
            pid = fs::read_dir(&dir)
                .unwrap()
                .find_map(|entry| {
                    entry
                        .unwrap()
                        .file_name()
                        .to_string_lossy()
                        .strip_prefix("ready-")
                        .map(str::to_owned)
                })
                .unwrap_or_default();
            !pid.is_empty()
        });
        // A zombie still has a /proc entry. Keep the library host alive until
        // its background waiter has actually reaped the terminated opener.
        assert_eq!(
            unsafe { libc::kill(pid.parse().unwrap(), libc::SIGTERM) },
            0
        );
        wait_until(|| !std::path::Path::new(&format!("/proc/{pid}")).exists());
        return;
    }
    let mut fixture = OpenerFixture::new(true);
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.args([
        "--exact",
        "detached_opener_is_reaped_in_a_living_library_host",
        "--nocapture",
    ]);
    let rx = fixture.start(cmd, false);
    let output = rx
        .recv_timeout(Duration::from_secs(15))
        .expect("library host timed out");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(fixture.records(1), ["1\nhttps://one.example\nEOF\n"]);
}

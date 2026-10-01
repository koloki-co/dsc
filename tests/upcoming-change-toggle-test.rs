// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! Functional coverage for `dsc upcoming-change enable|disable` against a
//! stateful mock of the Upcoming Changes list and toggle endpoints.

mod common;
use common::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

#[derive(Default)]
struct State {
    enabled: bool,
    deps_met: bool,
    ignore_toggle: bool,
    puts: Vec<(String, String, bool)>, // (path, body, saw xhr header)
}

fn start_mock(state: Arc<Mutex<State>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            handle(stream, &state);
        }
    });
    format!("http://{addr}")
}

fn handle(mut stream: TcpStream, state: &Mutex<State>) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() || request_line.trim().is_empty() {
        return;
    }
    let mut content_length = 0usize;
    let mut xhr = false;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() || line == "\r\n" || line == "\n" {
            break;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("content-length:") {
            content_length = v.trim().parse().unwrap_or(0);
        }
        if lower.starts_with("x-requested-with:") && lower.contains("xmlhttprequest") {
            xhr = true;
        }
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        let _ = reader.read_exact(&mut body);
    }
    let body = String::from_utf8_lossy(&body).to_string();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();

    let mut st = state.lock().unwrap();
    let (status, out) = if method == "GET"
        && path.starts_with("/admin/config/upcoming-changes.json")
    {
        (
            "HTTP/1.1 200 OK",
            format!(
                r#"{{"upcoming_changes":[{{"setting":"enable_generated_llms_txt","value":{},"depends_on_met":{},"upcoming_change":{{"status":"beta","enabled_for":"everyone"}}}}]}}"#,
                st.enabled, st.deps_met
            ),
        )
    } else if method == "PUT" && path.starts_with("/admin/config/upcoming-changes/toggle.json") {
        st.puts.push((path.clone(), body.clone(), xhr));
        if !st.ignore_toggle {
            st.enabled = body.contains("enabled=true");
        }
        ("HTTP/1.1 200 OK", r#"{"success":"OK"}"#.to_string())
    } else {
        ("HTTP/1.1 404 Not Found", "{}".to_string())
    };
    drop(st);
    let response = format!(
        "{status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}",
        out.len()
    );
    let _ = stream.write_all(response.as_bytes());
}

fn setup(
    enabled: bool,
    deps_met: bool,
    ignore_toggle: bool,
) -> (Arc<Mutex<State>>, TempDir, std::path::PathBuf) {
    let state = Arc::new(Mutex::new(State {
        enabled,
        deps_met,
        ignore_toggle,
        puts: vec![],
    }));
    let url = start_mock(Arc::clone(&state));
    let dir = TempDir::new().unwrap();
    let config = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"alpha\"\nbaseurl = \"{url}\"\napikey = \"k\"\napi_username = \"tester\"\n"
        ),
    );
    (state, dir, config)
}

const NAME: &str = "enable_generated_llms_txt";

#[test]
fn enable_sends_explicit_true_and_verifies() {
    let (state, _dir, config) = setup(false, true, false);
    let out = run_dsc(
        &["upcoming-change", "enable", "alpha", NAME, "-f", "json"],
        &config,
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["outcome"], "changed");
    assert_eq!(v["previous_value"], false);
    assert_eq!(v["value"], true);
    let st = state.lock().unwrap();
    assert_eq!(st.puts.len(), 1);
    assert!(
        st.puts[0]
            .1
            .contains("setting_name=enable_generated_llms_txt")
    );
    assert!(st.puts[0].1.contains("enabled=true"));
    assert!(st.puts[0].2, "toggle PUT must carry the XHR header");
}

#[test]
fn disable_sends_explicit_false() {
    let (state, _dir, config) = setup(true, true, false);
    let out = run_dsc(&["upcoming-change", "disable", "alpha", NAME], &config);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(state.lock().unwrap().puts[0].1.contains("enabled=false"));
}

#[test]
fn already_in_target_state_makes_no_put() {
    let (state, _dir, config) = setup(true, true, false);
    let out = run_dsc(&["upcoming-change", "enable", "alpha", NAME], &config);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("no change"));
    assert!(state.lock().unwrap().puts.is_empty());
}

#[test]
fn dry_run_reports_plan_without_put() {
    let (state, _dir, config) = setup(false, true, false);
    let out = run_dsc(
        &["--dry-run", "upcoming-change", "enable", "alpha", NAME],
        &config,
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("would enable"));
    assert!(state.lock().unwrap().puts.is_empty());
}

#[test]
fn enable_refuses_when_dependencies_unmet() {
    let (state, _dir, config) = setup(false, false, false);
    let out = run_dsc(&["upcoming-change", "enable", "alpha", NAME], &config);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("dependencies are not met"));
    assert!(state.lock().unwrap().puts.is_empty());
}

#[test]
fn verification_mismatch_is_nonzero() {
    let (_state, _dir, config) = setup(false, true, true);
    let out = run_dsc(&["upcoming-change", "enable", "alpha", NAME], &config);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("still reports"));
}

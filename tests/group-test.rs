// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

mod common;
use common::*;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tempfile::TempDir;

struct MockResponse {
    status: &'static str,
    body: &'static str,
}

fn start_mock(
    responses: Vec<MockResponse>,
) -> (String, Arc<Mutex<Vec<String>>>, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock listener");
    listener
        .set_nonblocking(true)
        .expect("configure mock listener");
    let addr = listener.local_addr().expect("mock addr");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let thread_requests = Arc::clone(&requests);
    let handle = std::thread::spawn(move || {
        let mut responses = VecDeque::from(responses);
        while let Some(response) = responses.pop_front() {
            let deadline = Instant::now() + Duration::from_secs(10);
            let stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "missing mock request");
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("accept mock request: {error}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("set mock read timeout");
            stream
                .set_write_timeout(Some(Duration::from_secs(5)))
                .expect("set mock write timeout");
            handle_request(stream, response, &thread_requests);
        }
    });
    (format!("http://{addr}"), requests, handle)
}

fn handle_request(mut stream: TcpStream, response: MockResponse, requests: &Mutex<Vec<String>>) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone mock stream"));
    let mut request_line = String::new();
    reader.read_line(&mut request_line).expect("request line");
    requests
        .lock()
        .expect("request log")
        .push(request_line.trim().to_string());
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).expect("request header");
        if line == "\r\n" || line == "\n" || line.is_empty() {
            break;
        }
    }
    let raw = format!(
        "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response.status,
        response.body.len(),
        response.body
    );
    stream.write_all(raw.as_bytes()).expect("mock response");
}

fn config_for(url: &str, dir: &TempDir) -> std::path::PathBuf {
    write_temp_config(
        dir,
        &format!(
            "[[discourse]]\nname = \"mock\"\nbaseurl = \"{url}\"\napikey = \"key\"\napi_username = \"system\"\n"
        ),
    )
}

#[test]
fn group_destroy_resolves_then_deletes_nonautomatic_group() {
    let responses = vec![
        MockResponse {
            status: "200 OK",
            body: r#"{"group":{"id":41,"name":"editors","automatic":false}}"#,
        },
        MockResponse {
            status: "200 OK",
            body: r#"{"success":"OK"}"#,
        },
    ];
    let (url, requests, handle) = start_mock(responses);
    let dir = TempDir::new().expect("tempdir");
    let output = run_dsc(
        &["group", "destroy", "mock", "41", "--format", "json"],
        &config_for(&url, &dir),
    );
    handle.join().expect("mock thread");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse structured result");
    assert_eq!(
        result,
        serde_json::json!({ "id": 41, "name": "editors", "action": "deleted" })
    );
    assert_eq!(
        requests.lock().expect("request log").as_slice(),
        [
            "GET /groups/41.json HTTP/1.1",
            "DELETE /admin/groups/41.json HTTP/1.1"
        ]
    );
}

#[test]
fn group_destroy_refuses_automatic_group_before_delete() {
    let responses = vec![MockResponse {
        status: "200 OK",
        body: r#"{"group":{"id":1,"name":"admins","automatic":true}}"#,
    }];
    let (url, requests, handle) = start_mock(responses);
    let dir = TempDir::new().expect("tempdir");
    let output = run_dsc(&["group", "destroy", "mock", "1"], &config_for(&url, &dir));
    handle.join().expect("mock thread");

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("group 1 (\"admins\") is an automatic group and cannot be deleted")
    );
    assert_eq!(
        requests.lock().expect("request log").as_slice(),
        ["GET /groups/1.json HTTP/1.1"]
    );
}

#[test]
fn group_destroy_structured_dry_run_reports_request_without_deleting() {
    let responses = vec![MockResponse {
        status: "200 OK",
        body: r#"{"group":{"id":41,"name":"editors","automatic":false}}"#,
    }];
    let (url, requests, handle) = start_mock(responses);
    let dir = TempDir::new().expect("tempdir");
    let output = run_dsc(
        &["-n", "group", "destroy", "mock", "41", "--format", "json"],
        &config_for(&url, &dir),
    );
    handle.join().expect("mock thread");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse structured plan");
    assert_eq!(
        result,
        serde_json::json!({
            "id": 41,
            "name": "editors",
            "action": "planned",
            "request": "DELETE /admin/groups/41.json"
        })
    );
    assert_eq!(
        requests.lock().expect("request log").as_slice(),
        ["GET /groups/41.json HTTP/1.1"]
    );
}

#[test]
fn group_destroy_rejects_zero_id_before_network_access() {
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        "[[discourse]]\nname = \"mock\"\nbaseurl = \"http://127.0.0.1:1\"\napikey = \"key\"\napi_username = \"system\"\n",
    );
    let output = run_dsc(&["group", "destroy", "mock", "0"], &config_path);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("0 is not in 1.."));
}

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn group_list() {
    let Some(test) = test_discourse() else {
        return;
    };
    vprintln("e2e_group_list: list groups");

    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n",
            test.name, test.baseurl, test.apikey, test.api_username
        ),
    );
    let output = run_dsc(&["group", "list", &test.name], &config_path);
    assert!(output.status.success(), "group list failed");
}

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn group_info() {
    let Some(test) = test_discourse() else {
        return;
    };
    let Some(group_id) = test.test_group_id else {
        return;
    };
    vprintln("e2e_group_info: fetch group info");

    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n",
            test.name, test.baseurl, test.apikey, test.api_username
        ),
    );
    let output = run_dsc(
        &["group", "info", &test.name, &group_id.to_string()],
        &config_path,
    );
    assert!(output.status.success(), "group info failed");
}

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn group_info_with_defaults() {
    let Some(test) = test_discourse() else {
        return;
    };
    let Some(group_id) = test.test_group_id else {
        return;
    };
    vprintln("e2e_group_info_with_defaults: fetch group info including notification defaults");

    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n",
            test.name, test.baseurl, test.apikey, test.api_username
        ),
    );
    let output = run_dsc(
        &[
            "group",
            "info",
            &test.name,
            &group_id.to_string(),
            "--with-defaults",
        ],
        &config_path,
    );
    assert!(output.status.success(), "group info --with-defaults failed");
}

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn group_members() {
    let Some(test) = test_discourse() else {
        return;
    };
    let Some(group_id) = test.test_group_id else {
        return;
    };
    vprintln("e2e_group_members: fetch group members");

    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n",
            test.name, test.baseurl, test.apikey, test.api_username
        ),
    );
    let output = run_dsc(
        &["group", "members", &test.name, &group_id.to_string()],
        &config_path,
    );
    assert!(output.status.success(), "group members failed");
}

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn group_copy() {
    let Some(test) = test_discourse() else {
        return;
    };
    let Some(group_id) = test.test_group_id else {
        return;
    };
    vprintln("e2e_group_copy: dry-run copy group on one forum");

    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n",
            test.name, test.baseurl, test.apikey, test.api_username
        ),
    );
    let output = run_dsc(
        &[
            "-n",
            "group",
            "copy",
            &test.name,
            &group_id.to_string(),
            "--target",
            &test.name,
        ],
        &config_path,
    );
    assert!(
        output.status.success(),
        "group copy --dry-run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("[dry-run]") && stdout.contains("would create group"),
        "expected dry-run group copy plan, got: {stdout}"
    );
}

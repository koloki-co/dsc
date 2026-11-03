// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

mod common;
use common::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tempfile::TempDir;

struct MockResponse {
    body: &'static str,
}

type RequestLog = Arc<Mutex<Vec<(String, String)>>>;

const MOVEMENTS: &[&str] = &[
    "Advocacy",
    "Community organising",
    "Co-operative",
    "Disability rights",
    "Education",
    "Environmental justice",
    "Faith community",
    "Food sovereignty",
    "Housing",
    "Labour union",
    "LGBTQ+ rights",
    "Migrant justice",
    "Mutual aid",
    "Peace building",
    "Public health",
    "Racial justice",
    "Rural development",
    "Student movement",
    "Women's rights",
    "Côte d'Ivoire solidarity",
    "日本の市民運動",
];

fn start_mock(responses: Vec<MockResponse>) -> (String, RequestLog, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock listener");
    listener.set_nonblocking(true).expect("configure listener");
    let address = listener.local_addr().expect("mock address");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let thread_requests = Arc::clone(&requests);
    let handle = std::thread::spawn(move || {
        for response in responses {
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
            handle_request(stream, response, &thread_requests);
        }
    });
    (format!("http://{address}"), requests, handle)
}

fn handle_request(
    mut stream: TcpStream,
    response: MockResponse,
    requests: &Mutex<Vec<(String, String)>>,
) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone mock stream"));
    let mut request_line = String::new();
    reader.read_line(&mut request_line).expect("request line");
    let mut content_length = 0;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).expect("request header");
        if line == "\r\n" || line == "\n" || line.is_empty() {
            break;
        }
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length: ") {
            content_length = value.trim().parse().expect("content length");
        }
    }
    let mut body = vec![0; content_length];
    reader.read_exact(&mut body).expect("request body");
    requests.lock().expect("request log").push((
        request_line.trim().to_string(),
        String::from_utf8(body).expect("UTF-8 body"),
    ));

    let raw = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
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

fn write_snapshot(dir: &TempDir, source: &str) -> std::path::PathBuf {
    let path = dir.path().join("user-fields.yaml");
    let options = MOVEMENTS
        .iter()
        .map(|option| format!("      - {option}\n"))
        .collect::<String>();
    std::fs::write(
        &path,
        format!(
            "version: 1\nsource: {source}\nuser_fields:\n  - name: Movements\n    description: Which movements do you work with?\n    field_type: multiselect\n    editable: true\n    requirement: on_signup\n    show_on_profile: false\n    show_on_signup: true\n    show_on_user_card: false\n    searchable: false\n    options:\n{options}"
        ),
    )
    .expect("write snapshot");
    path
}

#[test]
fn push_creates_a_new_field_in_the_verified_envelope() {
    let responses = vec![
        MockResponse {
            body: r#"{"user_fields":[]}"#,
        },
        MockResponse {
            body: r#"{"user_field":{"id":42,"name":"Movements"}}"#,
        },
    ];
    let (url, requests, handle) = start_mock(responses);
    let dir = TempDir::new().expect("tempdir");
    let snapshot = write_snapshot(&dir, &url);
    let config = config_for(&url, &dir);

    let output = run_dsc(
        &[
            "user-field",
            "push",
            "mock",
            snapshot.to_str().expect("path"),
        ],
        &config,
    );
    handle.join().expect("mock thread");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let requests = requests.lock().expect("request log");
    assert_eq!(requests[0].0, "GET /admin/config/user_fields.json HTTP/1.1");
    assert_eq!(
        requests[1].0,
        "POST /admin/config/user_fields.json HTTP/1.1"
    );
    let payload: serde_json::Value = serde_json::from_str(&requests[1].1).expect("JSON payload");
    assert_eq!(payload["user_field"]["name"], "Movements");
    assert_eq!(
        payload["user_field"]["options"],
        serde_json::json!(MOVEMENTS)
    );
    assert!(payload["user_field"].get("id").is_none());
}

#[test]
fn dry_run_plans_creation_without_posting() {
    let (url, requests, handle) = start_mock(vec![MockResponse {
        body: r#"{"user_fields":[]}"#,
    }]);
    let dir = TempDir::new().expect("tempdir");
    let snapshot = write_snapshot(&dir, &url);
    let config = config_for(&url, &dir);

    let output = run_dsc(
        &[
            "--dry-run",
            "user-field",
            "push",
            "mock",
            snapshot.to_str().expect("path"),
        ],
        &config,
    );
    handle.join().expect("mock thread");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&format!("target: \"{url}\"")));
    assert!(stdout.contains("+ create: \"Movements\""));
    assert!(stdout.contains("description: \"Which movements do you work with?\""));
    assert!(stdout.contains("field_type: multiselect"));
    assert!(stdout.contains("editable: true"));
    assert!(stdout.contains("requirement: on_signup"));
    assert!(stdout.contains("show_on_profile: false"));
    assert!(stdout.contains("show_on_signup: true"));
    assert!(stdout.contains("show_on_user_card: false"));
    assert!(stdout.contains("searchable: false"));
    for option in MOVEMENTS {
        assert!(stdout.contains(&serde_json::to_string(option).expect("JSON option")));
    }
    assert_eq!(requests.lock().expect("request log").len(), 1);
}

#[test]
fn pull_one_field_uses_the_show_endpoint_and_writes_one_definition() {
    let (url, requests, handle) = start_mock(vec![MockResponse {
        body: r#"{"user_field":{"id":42,"name":"Movements","description":"Which movements do you work with?","field_type":"multiselect","editable":true,"requirement":"on_signup","show_on_profile":false,"show_on_signup":true,"show_on_user_card":false,"searchable":false,"options":["Advocacy","日本の市民運動"]}}"#,
    }]);
    let dir = TempDir::new().expect("tempdir");
    let snapshot = dir.path().join("movements.yaml");
    let config = config_for(&url, &dir);

    let output = run_dsc(
        &[
            "ucf",
            "pull",
            "mock",
            "42",
            snapshot.to_str().expect("path"),
        ],
        &config,
    );
    handle.join().expect("mock thread");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        requests.lock().expect("request log")[0].0,
        "GET /admin/config/user_fields/42.json HTTP/1.1"
    );
    let snapshot = std::fs::read_to_string(&snapshot).expect("read snapshot");
    assert!(snapshot.contains("id: 42"));
    assert!(snapshot.contains("name: Movements"));
    assert!(snapshot.contains("- 日本の市民運動"));
}

#[test]
fn pull_path_remains_a_full_export_for_compatibility() {
    let (url, requests, handle) = start_mock(vec![MockResponse {
        body: r#"{"user_fields":[]}"#,
    }]);
    let dir = TempDir::new().expect("tempdir");
    let snapshot = dir.path().join("user-fields.yaml");
    let config = config_for(&url, &dir);

    let output = run_dsc(
        &[
            "user-field",
            "pull",
            "mock",
            snapshot.to_str().expect("path"),
        ],
        &config,
    );
    handle.join().expect("mock thread");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        requests.lock().expect("request log")[0].0,
        "GET /admin/config/user_fields.json HTTP/1.1"
    );
    let snapshot = std::fs::read_to_string(&snapshot).expect("read snapshot");
    assert!(snapshot.contains("user_fields: []"));
}

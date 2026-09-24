// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

mod common;
use common::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tempfile::TempDir;

fn start_mock(
    queries: Vec<(i64, &str)>,
    marker: &str,
    run_delay: Duration,
) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock listener");
    let addr = listener.local_addr().expect("mock addr");
    let requests = Arc::new(AtomicUsize::new(0));
    let thread_requests = Arc::clone(&requests);
    let marker = marker.to_string();
    let queries: Vec<(i64, String)> = queries
        .into_iter()
        .map(|(id, name)| (id, name.to_string()))
        .collect();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            handle_mock(stream, &queries, &marker, run_delay, &thread_requests);
        }
    });
    (format!("http://{addr}"), requests)
}

fn handle_mock(
    mut stream: TcpStream,
    queries: &[(i64, String)],
    marker: &str,
    run_delay: Duration,
    requests: &AtomicUsize,
) {
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(stream) => stream,
        Err(_) => return,
    });
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() || request_line.trim().is_empty() {
        return;
    }
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() || line == "\r\n" || line == "\n" {
            break;
        }
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            content_length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        let _ = reader.read_exact(&mut body);
    }

    requests.fetch_add(1, Ordering::Relaxed);
    let path = request_line.split_whitespace().nth(1).unwrap_or("");
    let response_body = if path.starts_with("/admin/plugins/discourse-data-explorer/queries.json") {
        let rows = queries
            .iter()
            .map(|(id, name)| serde_json::json!({"id": id, "name": name}))
            .collect::<Vec<_>>();
        serde_json::json!({
            "queries": rows,
            "total_rows_queries": rows.len(),
            "load_more_queries": null
        })
        .to_string()
    } else if path.contains("/run.json") {
        std::thread::sleep(run_delay);
        serde_json::json!({
            "success": true,
            "errors": [],
            "params": {},
            "duration": 1.0,
            "columns": ["forum"],
            "rows": [[marker]]
        })
        .to_string()
    } else {
        "{}".to_string()
    };
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response_body.len(),
        response_body
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn fleet_config(dir: &TempDir, alpha_url: &str, beta_url: &str) -> std::path::PathBuf {
    write_temp_config(
        dir,
        &format!(
            "[[discourse]]\nname = \"alpha\"\nbaseurl = \"{alpha_url}\"\napikey = \"k\"\napi_username = \"tester\"\ntags = [\"production\"]\n\n[[discourse]]\nname = \"beta\"\nbaseurl = \"{beta_url}\"\napikey = \"k\"\napi_username = \"tester\"\ntags = [\"production\"]\n"
        ),
    )
}

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn explorer_lists_and_runs_builtin_query() {
    let Some(test) = test_discourse() else {
        return;
    };
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n",
            test.name, test.baseurl, test.apikey, test.api_username
        ),
    );

    let list = run_dsc(
        &["explorer", "list", &test.name, "--format", "json"],
        &config_path,
    );
    if !list.status.success() {
        let stderr = String::from_utf8_lossy(&list.stderr);
        if stderr.contains("Data Explorer may be disabled") {
            eprintln!("Data Explorer is disabled on {}; skipping run", test.name);
            return;
        }
    }
    assert!(
        list.status.success(),
        "explorer list failed: {}",
        String::from_utf8_lossy(&list.stderr)
    );
    let catalogue: serde_json::Value =
        serde_json::from_slice(&list.stdout).expect("explorer list JSON");
    assert!(
        catalogue["queries"]
            .as_array()
            .is_some_and(|rows| !rows.is_empty())
    );

    let run = run_dsc(
        &[
            "explorer", "run", &test.name, "-1", "--limit", "1", "--format", "json",
        ],
        &config_path,
    );
    assert!(
        run.status.success(),
        "explorer run failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&run.stdout).expect("explorer run JSON");
    assert_eq!(result["success"], true);
    assert!(result["columns"].is_array());
    assert!(result["rows"].is_array());
}

#[test]
fn explorer_runs_an_exact_query_name_across_the_fleet_in_config_order() {
    let (alpha_url, alpha_requests) = start_mock(
        vec![(7, "Shared Audit"), (8, "Shared Audit Old")],
        "alpha",
        Duration::from_millis(100),
    );
    let (beta_url, beta_requests) = start_mock(vec![(42, "Shared Audit")], "beta", Duration::ZERO);
    let dir = TempDir::new().expect("tempdir");
    let config = fleet_config(&dir, &alpha_url, &beta_url);

    let output = run_dsc(
        &[
            "explorer",
            "run",
            "--all",
            "--query-name",
            "Shared Audit",
            "--params",
            r#"{"days":90}"#,
            "--format",
            "json",
        ],
        &config,
    );
    assert!(
        output.status.success(),
        "fleet explorer run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let rows: Vec<serde_json::Value> =
        serde_json::from_slice(&output.stdout).expect("fleet result JSON");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["forum"], "alpha");
    assert_eq!(rows[0]["query_id"], 7);
    assert_eq!(rows[0]["result"]["rows"][0][0], "alpha");
    assert_eq!(rows[1]["forum"], "beta");
    assert_eq!(rows[1]["query_id"], 42);
    assert_eq!(rows[1]["result"]["rows"][0][0], "beta");
    assert_eq!(alpha_requests.load(Ordering::Relaxed), 2);
    assert_eq!(beta_requests.load(Ordering::Relaxed), 2);
}

#[test]
fn explorer_runs_an_exact_query_name_on_one_forum_with_the_existing_result_shape() {
    let (alpha_url, alpha_requests) =
        start_mock(vec![(7, "Shared Audit")], "alpha", Duration::ZERO);
    let (beta_url, beta_requests) = start_mock(vec![(42, "Shared Audit")], "beta", Duration::ZERO);
    let dir = TempDir::new().expect("tempdir");
    let config = fleet_config(&dir, &alpha_url, &beta_url);

    let output = run_dsc(
        &[
            "explorer",
            "run",
            "alpha",
            "--query-name",
            "Shared Audit",
            "--format",
            "json",
        ],
        &config,
    );
    assert!(output.status.success());
    let result: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("single-forum result JSON");
    assert_eq!(result["success"], true);
    assert_eq!(result["rows"][0][0], "alpha");
    assert!(result.get("forum").is_none());
    assert_eq!(alpha_requests.load(Ordering::Relaxed), 2);
    assert_eq!(beta_requests.load(Ordering::Relaxed), 0);
}

#[test]
fn explorer_fleet_run_retains_exact_name_failures_and_successes() {
    let (alpha_url, _) = start_mock(vec![(7, "Shared Audit")], "alpha", Duration::ZERO);
    let (beta_url, _) = start_mock(vec![(42, "Shared Audit Extended")], "beta", Duration::ZERO);
    let dir = TempDir::new().expect("tempdir");
    let config = fleet_config(&dir, &alpha_url, &beta_url);

    let output = run_dsc(
        &[
            "explorer",
            "run",
            "--all",
            "--query-name",
            "Shared Audit",
            "--format",
            "json",
        ],
        &config,
    );
    assert!(!output.status.success());
    let rows: Vec<serde_json::Value> =
        serde_json::from_slice(&output.stdout).expect("partial fleet result JSON");
    assert_eq!(rows[0]["forum"], "alpha");
    assert_eq!(rows[0]["result"]["success"], true);
    assert_eq!(rows[1]["forum"], "beta");
    assert!(rows[1]["error"].as_str().unwrap().contains("exact name"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("failed on 1 of 2 forum(s)"));
}

#[test]
fn explorer_fleet_run_rejects_duplicate_exact_names() {
    let (alpha_url, _) = start_mock(
        vec![(7, "Shared Audit"), (9, "Shared Audit")],
        "alpha",
        Duration::ZERO,
    );
    let (beta_url, _) = start_mock(vec![(42, "Shared Audit")], "beta", Duration::ZERO);
    let dir = TempDir::new().expect("tempdir");
    let config = fleet_config(&dir, &alpha_url, &beta_url);

    let output = run_dsc(
        &[
            "explorer",
            "run",
            "--all",
            "--query-name",
            "Shared Audit",
            "--format",
            "json",
        ],
        &config,
    );
    assert!(!output.status.success());
    let rows: Vec<serde_json::Value> =
        serde_json::from_slice(&output.stdout).expect("duplicate-name fleet result JSON");
    assert!(rows[0]["error"].as_str().unwrap().contains("IDs 7, 9"));
    assert_eq!(rows[1]["query_id"], 42);
}

#[test]
fn explorer_fleet_dry_run_does_not_contact_forums() {
    let (alpha_url, alpha_requests) =
        start_mock(vec![(7, "Shared Audit")], "alpha", Duration::ZERO);
    let (beta_url, beta_requests) = start_mock(vec![(42, "Shared Audit")], "beta", Duration::ZERO);
    let dir = TempDir::new().expect("tempdir");
    let config = fleet_config(&dir, &alpha_url, &beta_url);

    let output = run_dsc(
        &[
            "--dry-run",
            "explorer",
            "run",
            "--all",
            "--query-name",
            "Shared Audit",
        ],
        &config,
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("alpha"));
    assert!(stdout.contains("beta"));
    assert!(stdout.contains("Shared Audit"));
    assert_eq!(alpha_requests.load(Ordering::Relaxed), 0);
    assert_eq!(beta_requests.load(Ordering::Relaxed), 0);
}

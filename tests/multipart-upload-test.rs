// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! Offline wire-level coverage of streaming retries and emoji endpoint discovery.

use dsc::api::DiscourseClient;
use dsc::config::DiscourseConfig;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const CURRENT: &str = "/admin/config/emoji.json";
const LEGACY_JSON: &str = "/admin/customize/emojis.json";
const LEGACY_PATH: &str = "/admin/customize/emojis";

fn payload() -> Vec<u8> {
    (0..128 * 1024).map(|i| (i % 256) as u8).collect()
}

// A finite script proves request order/count as well as bytes. Timeouts ensure
// a missing request fails rather than leaving a mock thread blocked forever.
fn mock(steps: Vec<(&'static str, u16)>) -> (DiscourseClient, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let client = DiscourseClient::new(&DiscourseConfig {
        name: "mock".into(),
        baseurl: format!("http://{}", listener.local_addr().unwrap()),
        ..Default::default()
    })
    .unwrap();
    let thread = std::thread::spawn(move || {
        for (path, status) in steps {
            let deadline = Instant::now() + Duration::from_secs(15);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "missing request to {path}");
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(err) => panic!("accept: {err}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let mut request = line.split_whitespace();
            assert_eq!(request.next(), Some("POST"));
            assert_eq!(request.next().unwrap().split('?').next(), Some(path));
            let mut length = None;
            let mut boundary = None;
            loop {
                line.clear();
                assert!(reader.read_line(&mut line).unwrap() > 0);
                if line == "\r\n" {
                    break;
                }
                let (name, value) = line.split_once(':').unwrap();
                if name.eq_ignore_ascii_case("content-length") {
                    length = Some(value.trim().parse::<usize>().unwrap());
                }
                if name.eq_ignore_ascii_case("content-type") {
                    boundary = Some(
                        value
                            .trim()
                            .strip_prefix("multipart/form-data; boundary=")
                            .unwrap()
                            .to_string(),
                    );
                }
            }
            let mut body = vec![0; length.expect("known-length multipart")];
            reader.read_exact(&mut body).unwrap();
            let boundary = boundary.unwrap();
            assert!(body.starts_with(format!("--{boundary}\r\n").as_bytes()));
            assert!(body.ends_with(format!("\r\n--{boundary}--\r\n").as_bytes()));
            let (field, mime, text_fields): (&str, &str, &[(&str, &str)]) = match path {
                "/uploads.json" => ("file", "", &[("type", "composer"), ("synchronous", "true")]),
                "/admin/themes/import.json" => ("bundle", "", &[]),
                CURRENT => ("file", "Content-Type: image/png\r\n", &[("name", "sample")]),
                LEGACY_JSON | LEGACY_PATH => (
                    "emoji[image]",
                    "Content-Type: image/png\r\n",
                    &[("emoji[name]", "sample")],
                ),
                _ => panic!("unexpected path {path}"),
            };
            let mut expected = format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{field}\"; filename=\"sample.bin\"\r\n{mime}\r\n").into_bytes();
            expected.extend(payload());
            expected.extend_from_slice(format!("\r\n--{boundary}").as_bytes());
            assert!(
                body.windows(expected.len()).any(|part| part == expected),
                "complete file part missing at {path} ({status})"
            );
            for (name, value) in text_fields {
                let expected = format!(
                    "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n--{boundary}"
                );
                assert!(
                    body.windows(expected.len())
                        .any(|part| part == expected.as_bytes()),
                    "missing {name} at {path}"
                );
            }
            let response = r#"{"id":1,"url":"/uploads/sample.bin","original_filename":"sample.bin","filesize":131072}"#;
            write!(stream, "HTTP/1.1 {status} Mock\r\nContent-Type: application/json\r\nContent-Length: {}\r\nRetry-After: 1\r\nConnection: close\r\n\r\n{response}", response.len()).unwrap();
        }
    });
    (client, thread)
}

fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sample.bin");
    std::fs::write(&path, payload()).unwrap();
    (dir, path)
}

#[test]
fn client_remains_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<DiscourseClient>();
}

#[test]
fn uploads_send_complete_multipart_on_initial_attempt_and_429_retry() {
    let (_dir, path) = fixture();
    for endpoint in [
        "/uploads.json",
        "/admin/themes/import.json",
        CURRENT,
        LEGACY_JSON,
        LEGACY_PATH,
    ] {
        let mut steps = Vec::new();
        if matches!(endpoint, LEGACY_JSON | LEGACY_PATH) {
            steps.push((CURRENT, 404));
        }
        if endpoint == LEGACY_PATH {
            steps.push((LEGACY_JSON, 404));
        }
        steps.extend([(endpoint, 429), (endpoint, 200)]);
        let (client, server) = mock(steps);
        match endpoint {
            "/uploads.json" => {
                client.upload_file(&path, "composer").unwrap();
            }
            "/admin/themes/import.json" => {
                client.import_theme_bundle(&path).unwrap();
            }
            _ => client.upload_emoji(&path, "sample").unwrap(),
        }
        server.join().unwrap();
    }
}

#[test]
fn emoji_caches_each_successful_endpoint_across_client_clones() {
    let (_dir, path) = fixture();
    for endpoint in [CURRENT, LEGACY_JSON, LEGACY_PATH] {
        let mut steps = Vec::new();
        for candidate in [CURRENT, LEGACY_JSON, LEGACY_PATH] {
            if candidate == endpoint {
                break;
            }
            steps.push((candidate, 404));
        }
        steps.extend([(endpoint, 200), (endpoint, 200)]);
        let (client, server) = mock(steps);
        let clone = client.clone();
        client.upload_emoji(&path, "sample").unwrap();
        clone.upload_emoji(&path, "sample").unwrap();
        server.join().unwrap();
    }
}

#[test]
fn emoji_does_not_cache_or_fall_back_on_auth_validation_or_server_errors() {
    let (_dir, path) = fixture();
    for status in [401, 403, 422, 500] {
        let (client, server) = mock(vec![(CURRENT, 404), (LEGACY_JSON, status), (CURRENT, 200)]);
        let err = client.upload_emoji(&path, "sample").unwrap_err();
        assert!(err.to_string().contains(&status.to_string()));
        client.upload_emoji(&path, "sample").unwrap();
        server.join().unwrap();
    }
}

#[test]
fn emoji_cached_404_rediscovers_and_caches_a_working_endpoint() {
    let (_dir, path) = fixture();
    let (client, server) = mock(vec![
        (CURRENT, 404),
        (LEGACY_JSON, 200),
        (LEGACY_JSON, 404),
        (CURRENT, 200),
        (CURRENT, 200),
    ]);
    for _ in 0..3 {
        client.upload_emoji(&path, "sample").unwrap();
    }
    server.join().unwrap();
}

#[test]
fn emoji_cached_404_clears_cache_even_when_rediscovery_fails() {
    let (_dir, path) = fixture();
    let (client, server) = mock(vec![
        (CURRENT, 404),
        (LEGACY_JSON, 200),
        (LEGACY_JSON, 404),
        (CURRENT, 404),
        (LEGACY_PATH, 404),
        (CURRENT, 200),
    ]);
    client.upload_emoji(&path, "sample").unwrap();
    assert!(
        client
            .upload_emoji(&path, "sample")
            .unwrap_err()
            .to_string()
            .contains("404")
    );
    client.upload_emoji(&path, "sample").unwrap();
    server.join().unwrap();
}

#[test]
fn emoji_cached_errors_preserve_the_previously_successful_endpoint() {
    let (_dir, path) = fixture();
    for status in [401, 403, 422, 500] {
        let (client, server) = mock(vec![
            (CURRENT, 404),
            (LEGACY_JSON, 200),
            (LEGACY_JSON, status),
            (LEGACY_JSON, 200),
        ]);
        client.upload_emoji(&path, "sample").unwrap();
        assert!(client.upload_emoji(&path, "sample").is_err());
        client.upload_emoji(&path, "sample").unwrap();
        server.join().unwrap();
    }
}

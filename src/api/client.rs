// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use super::models::{AboutResponse, SiteResponse};
use super::rate_limit::{RETRY_BUFFER, parse_rate_limit_wait, summarize_rate_limit_body};
use crate::config::DiscourseConfig;
use crate::utils::normalize_baseurl;
use anyhow::{Context, Result, anyhow};
use reqwest::StatusCode;
use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{HeaderMap, HeaderValue};
use std::time::Duration;

const MAX_RATE_LIMIT_RETRIES: u32 = 5;

/// Cap on connection establishment; a dead/unreachable host should fail fast
/// rather than hang the CLI indefinitely.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Cap on total request time, generous enough for the largest routine
/// payloads (theme bundle/emoji uploads, full setting/topic pulls).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone)]
pub struct VersionInfo {
    pub version: Option<String>,
    pub commit: Option<String>,
}

/// HTTP client for the Discourse API.
#[derive(Clone)]
pub struct DiscourseClient {
    baseurl: String,
    client: Client,
}

impl DiscourseClient {
    /// Create a new Discourse API client.
    pub fn new(config: &DiscourseConfig) -> Result<Self> {
        let baseurl = normalize_baseurl(&config.baseurl);
        if baseurl.is_empty() {
            return Err(anyhow!(
                "missing baseurl for discourse {}; please set baseurl or check your config",
                config.name
            ));
        }

        let mut headers = HeaderMap::new();
        if let (Some(apikey), Some(api_username)) =
            (config.apikey.as_ref(), config.api_username.as_ref())
        {
            headers.insert(
                "Api-Key",
                HeaderValue::from_str(apikey).context("invalid api key")?,
            );
            headers.insert(
                "Api-Username",
                HeaderValue::from_str(api_username).context("invalid api username")?,
            );
        }

        let client = Client::builder()
            .default_headers(headers)
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .context("building http client")?;

        Ok(Self { baseurl, client })
    }

    /// Return the configured base URL.
    pub fn baseurl(&self) -> &str {
        &self.baseurl
    }

    /// Return the underlying reqwest client, for requests that need to
    /// opt out of the standard per-request timeout (e.g. bulk backup
    /// downloads that can legitimately take minutes).
    pub(crate) fn raw_client(&self) -> &Client {
        &self.client
    }

    pub(crate) fn get(&self, path: &str) -> Result<Response> {
        let url = format!("{}{}", self.baseurl, path);
        // Route GETs through `send_retrying` so a 429 response (whether
        // from Discourse's app-level rate limit or a fronting nginx)
        // is retried automatically. Crucial for the analytics command
        // which can fan out tens of GETs back-to-back.
        self.send_retrying(|| Ok(self.client.get(&url).timeout(REQUEST_TIMEOUT)))
    }

    pub(crate) fn post(&self, path: &str) -> Result<reqwest::blocking::RequestBuilder> {
        let url = format!("{}{}", self.baseurl, path);
        Ok(self.client.post(url).timeout(REQUEST_TIMEOUT))
    }

    pub(crate) fn put(&self, path: &str) -> Result<reqwest::blocking::RequestBuilder> {
        let url = format!("{}{}", self.baseurl, path);
        Ok(self.client.put(url).timeout(REQUEST_TIMEOUT))
    }

    pub(crate) fn delete(&self, path: &str) -> Result<reqwest::blocking::Response> {
        let url = format!("{}{}", self.baseurl, path);
        self.client
            .delete(url)
            .timeout(REQUEST_TIMEOUT)
            .send()
            .context("sending delete request")
    }

    pub(crate) fn delete_builder(&self, path: &str) -> Result<RequestBuilder> {
        let url = format!("{}{}", self.baseurl, path);
        Ok(self.client.delete(url))
    }

    /// Send a request, retrying up to 5 times on HTTP 429 responses.
    ///
    /// The `build` closure is called once per attempt and must produce a fresh
    /// `RequestBuilder` each time; this lets callers with non-cloneable bodies
    /// (e.g. multipart forms) participate in retries.
    pub(crate) fn send_retrying<F>(&self, mut build: F) -> Result<Response>
    where
        F: FnMut() -> Result<RequestBuilder>,
    {
        let mut attempt: u32 = 0;
        loop {
            let rb = build()?;
            let response = rb.send().context("sending request")?;
            if response.status() != StatusCode::TOO_MANY_REQUESTS {
                return Ok(response);
            }
            let headers = response.headers().clone();
            let body = response
                .text()
                .unwrap_or_else(|_| "<failed to read 429 body>".to_string());
            if attempt >= MAX_RATE_LIMIT_RETRIES {
                return Err(anyhow!(
                    "rate-limited after {} retries: {}",
                    MAX_RATE_LIMIT_RETRIES,
                    summarize_rate_limit_body(&body)
                ));
            }
            attempt += 1;
            let wait = parse_rate_limit_wait(&headers, &body) + RETRY_BUFFER;
            eprintln!(
                "Rate limited, waiting {}s (retry {}/{})",
                wait.as_secs(),
                attempt,
                MAX_RATE_LIMIT_RETRIES
            );
            std::thread::sleep(wait);
        }
    }

    /// Fetch the Discourse site title.
    pub fn fetch_site_title(&self) -> Result<String> {
        let site_json_error = match self.get("/site.json") {
            Ok(response) => {
                let status = response.status();
                let text = response.text().context("reading site.json response body")?;
                if status.is_success() {
                    let body: SiteResponse =
                        serde_json::from_str(&text).context("parsing site.json")?;
                    return Ok(body.site.title);
                }
                anyhow!("site.json request failed with {}", status)
            }
            Err(err) => err,
        };

        let response = self.get("/")?;
        let status = response.status();
        let html = response.text().context("reading site HTML")?;
        if !status.is_success() {
            return Err(anyhow!(
                "site title lookup failed (site.json error: {}; HTML request failed with {})",
                site_json_error,
                status
            ));
        }
        if let Some(title) = extract_html_title(&html) {
            return Ok(title);
        }
        Err(anyhow!(
            "site title lookup failed (site.json error: {}; HTML missing <title>)",
            site_json_error
        ))
    }

    /// Fetch the current Discourse version and commit hash.
    pub fn fetch_version_info(&self) -> Result<VersionInfo> {
        let mut version = None;
        let mut commit = None;
        let mut last_err = None;

        match self.get("/about.json") {
            Ok(response) => {
                let status = response.status();
                match response.json::<AboutResponse>() {
                    Ok(body) => {
                        if status.is_success() {
                            version = body.about.version.or(body.about.installed_version);
                        } else {
                            last_err = Some(anyhow!("about.json request failed with {}", status));
                        }
                    }
                    Err(err) => {
                        last_err = Some(anyhow!("reading about.json: {}", err));
                    }
                }
            }
            Err(err) => {
                last_err = Some(err);
            }
        }

        match self.get("/") {
            Ok(response) => {
                let status = response.status();
                let html = response.text().context("reading site HTML")?;
                if !status.is_success() {
                    last_err = Some(anyhow!("site HTML request failed with {}", status));
                } else if let Some(content) = extract_meta_content(&html, "generator") {
                    let (html_version, html_commit) = parse_generator_content(&content);
                    if version.is_none() {
                        version = html_version;
                    }
                    if commit.is_none() {
                        commit = html_commit;
                    }
                }
            }
            Err(err) => {
                last_err = Some(err);
            }
        }

        if version.is_none() && commit.is_none() {
            return Err(last_err.unwrap_or_else(|| anyhow!("version fetch failed")));
        }

        Ok(VersionInfo { version, commit })
    }

    /// Fetch the current Discourse version (best-effort).
    pub fn fetch_version(&self) -> Result<Option<String>> {
        Ok(self.fetch_version_info()?.version)
    }
}

fn extract_html_title(html: &str) -> Option<String> {
    let haystack = html.as_bytes();
    let mut lower = Vec::with_capacity(haystack.len());
    for &byte in haystack {
        lower.push(byte.to_ascii_lowercase());
    }
    let open_tag = b"<title>";
    let close_tag = b"</title>";
    let start = find_subslice(&lower, open_tag)? + open_tag.len();
    let end = find_subslice(&lower[start..], close_tag)? + start;
    let title = String::from_utf8_lossy(&haystack[start..end])
        .trim()
        .to_string();
    if title.is_empty() { None } else { Some(title) }
}

fn extract_meta_content(html: &str, name: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let name_attr = format!("name=\"{}\"", name.to_ascii_lowercase());
    let name_attr_single = format!("name='{}'", name.to_ascii_lowercase());

    let mut start = 0;
    while let Some(pos) = lower[start..].find("<meta") {
        let tag_start = start + pos;
        let rest = &lower[tag_start..];
        let tag_end = rest.find('>')? + tag_start;
        let tag_lower = &lower[tag_start..tag_end];
        if tag_lower.contains(&name_attr) || tag_lower.contains(&name_attr_single) {
            let tag_original = &html[tag_start..tag_end];
            if let Some(value) = extract_attr_value(tag_original, "content") {
                return Some(value);
            }
        }
        start = tag_end + 1;
    }
    None
}

fn extract_attr_value(tag: &str, attr: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let attr_eq = format!("{}=", attr.to_ascii_lowercase());
    let pos = lower.find(&attr_eq)? + attr_eq.len();
    let rest = &tag[pos..];
    let mut chars = rest.chars();
    let quote = chars.next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let value: String = chars.take_while(|c| *c != quote).collect();
    if value.is_empty() { None } else { Some(value) }
}

fn parse_generator_content(content: &str) -> (Option<String>, Option<String>) {
    let mut version = None;
    let mut commit = None;

    if let Some(rest) = content.strip_prefix("Discourse ") {
        let ver = rest.split(" - ").next().map(|s| s.trim()).unwrap_or("");
        if !ver.is_empty() {
            version = Some(ver.to_string());
        }
    }

    if let Some(idx) = content.find("version ") {
        let tail = &content[idx + "version ".len()..];
        let hash = tail.split_whitespace().next().unwrap_or("");
        if !hash.is_empty() {
            commit = Some(hash.to_string());
        }
    }

    (version, commit)
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

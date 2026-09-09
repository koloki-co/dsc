// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use super::client::{DiscourseClient, ResponseBody};
use super::error::http_error;
use super::models::CustomEmoji;
use anyhow::{Context, Result, anyhow};
use reqwest::StatusCode;
use reqwest::blocking::Response;
use serde_json::Value;
use std::fs::File;
use std::path::Path;

/// Fixed probe order for the emoji-upload endpoint. Discourse's admin emoji
/// UI moved from `/admin/customize/emojis` to `/admin/customize/emojis.json`
/// to (current) `/admin/config/emoji.json` across versions; `dsc` supports
/// all three by falling back on 404. Represented as `u8` indices so
/// `DiscourseClient` can cache the discovered endpoint without depending on
/// an emoji-specific type.
const EMOJI_ENDPOINT_V2: u8 = 0;
const EMOJI_ENDPOINT_LEGACY_JSON: u8 = 1;
const EMOJI_ENDPOINT_LEGACY_PATH: u8 = 2;
const EMOJI_ENDPOINT_PROBE_ORDER: [u8; 3] = [
    EMOJI_ENDPOINT_V2,
    EMOJI_ENDPOINT_LEGACY_JSON,
    EMOJI_ENDPOINT_LEGACY_PATH,
];

fn make_emoji_form(
    emoji_path: &Path,
    emoji_name: &str,
    image_field: &'static str,
    name_field: &'static str,
) -> Result<reqwest::blocking::multipart::Form> {
    // Stream from an open file handle rather than buffering the whole image
    // with `fs::read`, since this closure is reopened on every 429 retry.
    let file =
        File::open(emoji_path).with_context(|| format!("reading {}", emoji_path.display()))?;
    let len = file
        .metadata()
        .with_context(|| format!("reading metadata for {}", emoji_path.display()))?
        .len();
    let filename = emoji_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("emoji.png")
        .to_string();
    let part = reqwest::blocking::multipart::Part::reader_with_length(file, len)
        .file_name(filename)
        .mime_str("image/png")
        .context("setting emoji mime")?;
    Ok(reqwest::blocking::multipart::Form::new()
        .part(image_field, part)
        .text(name_field, emoji_name.to_string()))
}

impl DiscourseClient {
    /// Upload a custom emoji. Retries on 429 via the shared client helper.
    pub fn upload_emoji(&self, emoji_path: &Path, emoji_name: &str) -> Result<()> {
        let upload_v2_json_path = emoji_admin_path("/admin/config/emoji.json");
        let upload_json_path = emoji_admin_path("/admin/customize/emojis.json");
        let upload_path = emoji_admin_path("/admin/customize/emojis");

        let request_endpoint = |idx: u8| -> Result<Response> {
            match idx {
                EMOJI_ENDPOINT_V2 => self.send_retrying(|| {
                    Ok(self
                        .post(&upload_v2_json_path)?
                        .multipart(make_emoji_form(emoji_path, emoji_name, "file", "name")?))
                }),
                EMOJI_ENDPOINT_LEGACY_JSON => self.send_retrying(|| {
                    Ok(self.post(&upload_json_path)?.multipart(make_emoji_form(
                        emoji_path,
                        emoji_name,
                        "emoji[image]",
                        "emoji[name]",
                    )?))
                }),
                _ => self.send_retrying(|| {
                    Ok(self.post(&upload_path)?.multipart(make_emoji_form(
                        emoji_path,
                        emoji_name,
                        "emoji[image]",
                        "emoji[name]",
                    )?))
                }),
            }
        };

        // Try the endpoint that last worked on this client first, so a bulk
        // upload only probes fallbacks while the endpoint is still unknown.
        let start = self
            .emoji_upload_endpoint
            .get()
            .unwrap_or(EMOJI_ENDPOINT_V2);
        let mut attempts: Vec<u8> = Vec::with_capacity(EMOJI_ENDPOINT_PROBE_ORDER.len());
        attempts.push(start);
        attempts.extend(
            EMOJI_ENDPOINT_PROBE_ORDER
                .into_iter()
                .filter(|&i| i != start),
        );

        let mut response = None;
        let mut iter = attempts.into_iter().peekable();
        while let Some(idx) = iter.next() {
            let candidate = request_endpoint(idx)?;
            let is_last = iter.peek().is_none();
            if candidate.status() != StatusCode::NOT_FOUND || is_last {
                if candidate.status() != StatusCode::NOT_FOUND {
                    self.emoji_upload_endpoint.set(Some(idx));
                }
                response = Some(candidate);
                break;
            }
        }
        let response = response.expect("attempts is non-empty; loop always yields a response");
        if !response.status().is_success() {
            let status = response.status();
            if matches!(
                status,
                StatusCode::NOT_FOUND | StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED
            ) {
                return Err(anyhow!(
                    "emoji upload failed with {} (requires an admin API key)",
                    status
                ));
            }
            let text = response
                .text_capped()
                .unwrap_or_else(|_| "<failed to read response body>".to_string());
            return Err(anyhow!("emoji upload failed with {}: {}", status, text));
        }
        Ok(())
    }

    /// List custom emojis.
    pub fn list_custom_emojis(&self) -> Result<Vec<CustomEmoji>> {
        if let Some(emojis) = self.list_admin_emojis()? {
            return Ok(emojis);
        }
        if let Some(emojis) = self.list_admin_config_emojis()? {
            return Ok(emojis);
        }
        self.list_public_emojis()
    }

    fn list_admin_emojis(&self) -> Result<Option<Vec<CustomEmoji>>> {
        let path = emoji_admin_path("/admin/customize/emojis.json");
        let response = self.get(&path)?;
        let status = response.status();
        let text = response
            .text_capped()
            .context("reading emoji list response")?;
        if !status.is_success() {
            if status == StatusCode::NOT_FOUND {
                return Ok(None);
            }
            return Err(http_error("emoji list request", status, &text));
        }
        let value: Value = serde_json::from_str(&text).context("parsing emoji list json")?;
        let emojis = if let Some(arr) = value.as_array() {
            extract_emojis_from_array(arr, self.baseurl())
        } else if let Some(val) = value.get("emojis") {
            extract_emojis_from_value(val, self.baseurl())
        } else if let Some(val) = value.get("custom_emoji") {
            extract_emojis_from_value(val, self.baseurl())
        } else if let Some(val) = value.get("custom") {
            extract_emojis_from_value(val, self.baseurl())
        } else if let Some(map) = value.as_object() {
            let mut out = Vec::new();
            extract_emojis_from_map(map, self.baseurl(), &mut out);
            out
        } else {
            Vec::new()
        };
        Ok(Some(emojis))
    }

    fn list_public_emojis(&self) -> Result<Vec<CustomEmoji>> {
        let response = self.get("/emoji.json")?;
        let status = response.status();
        let text = response
            .text_capped()
            .context("reading emoji.json response")?;
        if status == StatusCode::NOT_FOUND {
            return Ok(Vec::new());
        }
        if !status.is_success() {
            return Err(http_error("emoji.json request", status, &text));
        }
        let value: Value = serde_json::from_str(&text).context("parsing emoji.json")?;
        let baseurl = self.baseurl().trim_end_matches('/');
        let mut out = Vec::new();
        if let Some(val) = value.get("custom_emoji") {
            out.extend(extract_emojis_from_value(val, baseurl));
        }
        if let Some(val) = value.get("custom") {
            out.extend(extract_emojis_from_value(val, baseurl));
        }
        if out.is_empty()
            && let Some(val) = value.get("emoji")
        {
            out.extend(extract_emojis_from_value(val, baseurl));
        }
        Ok(out)
    }

    fn list_admin_config_emojis(&self) -> Result<Option<Vec<CustomEmoji>>> {
        let path = emoji_admin_path("/admin/config/emoji.json");
        let response = self.get(&path)?;
        let status = response.status();
        let text = response
            .text_capped()
            .context("reading admin config emoji response")?;
        if status == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(http_error("admin config emoji request", status, &text));
        }
        let value: Value =
            serde_json::from_str(&text).context("parsing admin config emoji json")?;
        if let Some(val) = value.get("emojis") {
            return Ok(Some(extract_emojis_from_value(val, self.baseurl())));
        }
        Ok(Some(extract_emojis_from_value(&value, self.baseurl())))
    }
}

fn emoji_admin_path(path: &str) -> String {
    let client_id = match std::env::var("DSC_EMOJI_CLIENT_ID") {
        Ok(value) => value,
        Err(_) => return path.to_string(),
    };
    let client_id = client_id.trim();
    if client_id.is_empty() || path.contains("client_id=") {
        return path.to_string();
    }
    let sep = if path.contains('?') { '&' } else { '?' };
    format!("{path}{sep}client_id={client_id}")
}

fn extract_emojis_from_array(emojis: &[Value], baseurl: &str) -> Vec<CustomEmoji> {
    let mut out = Vec::new();
    for item in emojis.iter() {
        let name = item.get("name").and_then(|v| v.as_str());
        let url = item
            .get("url")
            .and_then(|v| v.as_str())
            .or_else(|| item.get("image_url").and_then(|v| v.as_str()));
        if let (Some(name), Some(url)) = (name, url) {
            out.push(CustomEmoji {
                name: name.to_string(),
                url: normalize_emoji_url(baseurl, url),
            });
        }
    }
    out
}

fn extract_emojis_from_map(
    map: &serde_json::Map<String, Value>,
    baseurl: &str,
    out: &mut Vec<CustomEmoji>,
) {
    for (name, value) in map.iter() {
        let url = value
            .as_str()
            .or_else(|| value.get("url").and_then(|v| v.as_str()))
            .or_else(|| value.get("image_url").and_then(|v| v.as_str()))
            .or_else(|| value.get("path").and_then(|v| v.as_str()));
        if let Some(url) = url {
            out.push(CustomEmoji {
                name: name.to_string(),
                url: normalize_emoji_url(baseurl, url),
            });
        }
    }
}

fn extract_emojis_from_value(value: &Value, baseurl: &str) -> Vec<CustomEmoji> {
    match value {
        Value::Array(arr) => extract_emojis_from_array(arr, baseurl),
        Value::Object(map) => {
            let name = map.get("name").and_then(|v| v.as_str());
            let url = map
                .get("url")
                .and_then(|v| v.as_str())
                .or_else(|| map.get("image_url").and_then(|v| v.as_str()))
                .or_else(|| map.get("path").and_then(|v| v.as_str()));
            if let (Some(name), Some(url)) = (name, url) {
                return vec![CustomEmoji {
                    name: name.to_string(),
                    url: normalize_emoji_url(baseurl, url),
                }];
            }
            let mut out = Vec::new();
            extract_emojis_from_map(map, baseurl, &mut out);
            out
        }
        _ => Vec::new(),
    }
}

fn normalize_emoji_url(baseurl: &str, url: &str) -> String {
    if url.starts_with("http://") || url.starts_with("https://") {
        url.to_string()
    } else if url.starts_with("//") {
        let scheme = if baseurl.starts_with("http://") {
            "http:"
        } else {
            "https:"
        };
        format!("{}{}", scheme, url)
    } else if url.starts_with('/') {
        format!("{}{}", baseurl, url)
    } else {
        format!("{}/{}", baseurl, url)
    }
}

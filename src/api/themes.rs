// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};
use std::path::Path;

use super::client::DiscourseClient;
use super::error::http_error;

impl DiscourseClient {
    /// List installed themes on the Discourse instance.
    pub fn list_themes(&self) -> Result<Value> {
        let response = self.get("/admin/themes.json")?;
        let status = response.status();
        let text = response.text().context("reading themes response body")?;
        if !status.is_success() {
            return Err(http_error("themes request", status, &text));
        }
        let value: Value = serde_json::from_str(&text).context("parsing themes response")?;
        Ok(value)
    }

    /// Fetch a single theme by ID.
    pub fn fetch_theme(&self, theme_id: u64) -> Result<Value> {
        let response = self.get(&format!("/admin/themes/{}.json", theme_id))?;
        let status = response.status();
        let text = response.text().context("reading theme response body")?;
        if !status.is_success() {
            return Err(http_error("theme request", status, &text));
        }
        let value: Value = serde_json::from_str(&text).context("parsing theme response")?;
        Ok(value)
    }

    /// Create a new theme and return its ID.
    pub fn create_theme(&self, theme: &Value) -> Result<u64> {
        let payload = json!({ "theme": theme });
        let response =
            self.send_retrying(|| Ok(self.post("/admin/themes.json")?.json(&payload)))?;
        let status = response.status();
        let text = response.text().context("reading create theme response")?;
        if !status.is_success() {
            return Err(http_error("create theme request", status, &text));
        }
        let value: Value = serde_json::from_str(&text).context("parsing create theme response")?;
        let id = value
            .get("theme")
            .and_then(|v| v.get("id"))
            .or_else(|| value.get("id"))
            .and_then(|v| v.as_u64())
            .ok_or_else(|| anyhow!("missing theme id in create response"))?;
        Ok(id)
    }

    /// Delete a theme by ID.
    pub fn delete_theme(&self, theme_id: u64) -> Result<()> {
        let response = self.delete(&format!("/admin/themes/{}.json", theme_id))?;
        let status = response.status();
        let text = response.text().context("reading delete theme response")?;
        if !status.is_success() {
            return Err(http_error("delete theme request", status, &text));
        }
        Ok(())
    }

    /// Update an existing theme.
    pub fn update_theme(&self, theme_id: u64, theme: &Value) -> Result<()> {
        let payload = json!({ "theme": theme });
        let path = format!("/admin/themes/{}.json", theme_id);
        let response = self.send_retrying(|| Ok(self.put(&path)?.json(&payload)))?;
        let status = response.status();
        let text = response.text().context("reading update theme response")?;
        if !status.is_success() {
            return Err(http_error("update theme request", status, &text));
        }
        Ok(())
    }

    /// Set a single theme/component setting via
    /// `PUT /admin/themes/:id/setting.json` with `name` + `value` form fields.
    /// For JSON-schema list settings, `value` is the JSON text as a string
    /// (the caller passes it through verbatim).
    pub fn set_theme_setting(&self, theme_id: u64, name: &str, value: &str) -> Result<()> {
        let path = format!("/admin/themes/{}/setting.json", theme_id);
        let payload = [("name", name), ("value", value)];
        let response = self.send_retrying(|| Ok(self.put(&path)?.form(&payload)))?;
        let status = response.status();
        let text = response.text().context("reading theme setting response")?;
        if !status.is_success() {
            return Err(http_error("theme setting update request", status, &text));
        }
        Ok(())
    }

    /// Flip a boolean flag on a theme via `PUT /admin/themes/:id.json` and
    /// return the updated theme JSON. Used for the remote-theme lifecycle:
    /// `remote_check` refreshes `commits_behind` without pulling, and
    /// `remote_update` pulls the latest upstream commit.
    pub fn put_theme_flag(&self, theme_id: u64, flag: &str) -> Result<Value> {
        let payload = json!({ "theme": { flag: true } });
        let path = format!("/admin/themes/{}.json", theme_id);
        let response = self.send_retrying(|| Ok(self.put(&path)?.json(&payload)))?;
        let status = response.status();
        let text = response.text().context("reading theme flag response")?;
        if !status.is_success() {
            return Err(http_error("theme flag update request", status, &text));
        }
        let value: Value = serde_json::from_str(&text).context("parsing theme flag response")?;
        Ok(value)
    }

    /// Import a theme/component from a git repo via
    /// `POST /admin/themes/import.json`. `remote` may embed credentials for a
    /// private repo (`https://user:token@host/...`). Returns the created theme
    /// JSON. Retries only on 429 (which means the import didn't run), so a slow
    /// clone won't double-import.
    pub fn import_theme_remote(&self, remote: &str, branch: Option<&str>) -> Result<Value> {
        let mut form: Vec<(&str, &str)> = vec![("remote", remote)];
        if let Some(b) = branch.filter(|b| !b.is_empty()) {
            form.push(("branch", b));
        }
        let response =
            self.send_retrying(|| Ok(self.post("/admin/themes/import.json")?.form(&form)))?;
        let status = response.status();
        let text = response.text().context("reading theme import response")?;
        if !status.is_success() {
            return Err(http_error("theme import request", status, &text));
        }
        serde_json::from_str(&text).context("parsing theme import response")
    }

    /// Import a theme/component from a local bundle file (`.tar.gz`/zip export)
    /// via `POST /admin/themes/import.json` with the `bundle` multipart part.
    pub fn import_theme_bundle(&self, file: &Path) -> Result<Value> {
        let make_form = || -> Result<reqwest::blocking::multipart::Form> {
            let bytes =
                std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
            let filename = file
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or_else(|| anyhow!("theme bundle path missing filename: {}", file.display()))?
                .to_string();
            let part = reqwest::blocking::multipart::Part::bytes(bytes).file_name(filename);
            Ok(reqwest::blocking::multipart::Form::new().part("bundle", part))
        };
        let response = self.send_retrying(|| {
            Ok(self
                .post("/admin/themes/import.json")?
                .multipart(make_form()?))
        })?;
        let status = response.status();
        let text = response
            .text()
            .context("reading theme bundle import response")?;
        if !status.is_success() {
            return Err(http_error("theme bundle import request", status, &text));
        }
        serde_json::from_str(&text).context("parsing theme bundle import response")
    }
}

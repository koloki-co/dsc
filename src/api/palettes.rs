// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use anyhow::{Context, Result, anyhow};
use serde_json::Value;
use std::collections::BTreeMap;

use super::client::DiscourseClient;
use super::error::http_error;

impl DiscourseClient {
    /// List color schemes (palettes) available on the Discourse instance.
    pub fn list_color_schemes(&self) -> Result<Value> {
        let response = self.get("/admin/color_schemes.json")?;
        let status = response.status();
        let text = response
            .text()
            .context("reading color schemes response body")?;
        if !status.is_success() {
            return Err(http_error("color schemes request", status, &text));
        }
        let value: Value = serde_json::from_str(&text).context("parsing color schemes response")?;
        Ok(value)
    }

    /// Fetch a color scheme (palette) by ID.
    pub fn fetch_color_scheme(&self, scheme_id: u64) -> Result<Value> {
        let response = self.get(&format!("/admin/color_schemes/{}.json", scheme_id))?;
        let status = response.status();
        let text = response
            .text()
            .context("reading color scheme response body")?;
        if !status.is_success() {
            return Err(http_error("color scheme request", status, &text));
        }
        let value: Value = serde_json::from_str(&text).context("parsing color scheme response")?;
        Ok(value)
    }

    /// Create a new color scheme (palette).
    pub fn create_color_scheme(
        &self,
        name: &str,
        colors: &BTreeMap<String, String>,
    ) -> Result<u64> {
        let mut payload: Vec<(String, String)> = Vec::new();
        payload.push(("color_scheme[name]".to_string(), name.to_string()));
        for (key, value) in colors {
            payload.push((format!("color_scheme[colors][{}]", key), value.to_string()));
        }
        let response =
            self.send_retrying(|| Ok(self.post("/admin/color_schemes.json")?.form(&payload)))?;
        let status = response.status();
        let text = response.text().context("reading color scheme response")?;
        if !status.is_success() {
            return Err(http_error("create color scheme request", status, &text));
        }
        let value: Value =
            serde_json::from_str(&text).context("parsing create color scheme response")?;
        let id = value
            .get("color_scheme")
            .and_then(|v| v.get("id"))
            .or_else(|| value.get("id"))
            .and_then(|v| v.as_u64())
            .ok_or_else(|| anyhow!("missing color scheme id in response"))?;
        Ok(id)
    }

    /// Update an existing color scheme (palette).
    pub fn update_color_scheme(
        &self,
        scheme_id: u64,
        name: Option<&str>,
        colors: &BTreeMap<String, String>,
    ) -> Result<()> {
        let mut payload: Vec<(String, String)> = Vec::new();
        if let Some(name) = name
            && !name.trim().is_empty()
        {
            payload.push(("color_scheme[name]".to_string(), name.to_string()));
        }
        for (key, value) in colors {
            payload.push((format!("color_scheme[colors][{}]", key), value.to_string()));
        }
        let path = format!("/admin/color_schemes/{}.json", scheme_id);
        let response = self.send_retrying(|| Ok(self.put(&path)?.form(&payload)))?;
        let status = response.status();
        let text = response.text().context("reading color scheme response")?;
        if !status.is_success() {
            return Err(http_error("update color scheme request", status, &text));
        }
        Ok(())
    }
}

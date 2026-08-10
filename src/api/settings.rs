// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use super::client::DiscourseClient;
use super::error::http_error;
use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A single site setting with its full metadata, as returned by
/// `GET /admin/site_settings.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteSettingDetail {
    pub setting: String,
    #[serde(default)]
    pub value: Value,
    #[serde(default)]
    pub default: Value,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub category: String,
    /// API field name is `type`; renamed to avoid the Rust keyword collision.
    #[serde(rename = "type", default)]
    pub setting_type: String,
}

impl DiscourseClient {
    /// Update a site setting by name (admin only).
    pub fn update_site_setting(&self, setting: &str, value: &str) -> Result<()> {
        let setting = setting.trim();
        if setting.is_empty() {
            return Err(anyhow!("missing site setting name for site setting update"));
        }
        if setting.chars().any(|ch| ch.is_whitespace() || ch == '/') {
            return Err(anyhow!(
                "site setting name contains invalid characters: {}",
                setting
            ));
        }
        let path = format!("/admin/site_settings/{}.json", setting);
        let payload = site_setting_form(setting, value);
        let response = self.send_retrying(|| Ok(self.put(&path)?.form(&payload)))?;
        let status = response.status();
        let text = response
            .text()
            .context("reading site setting update response")?;
        if !status.is_success() {
            return Err(http_error("update site setting request", status, &text));
        }
        Ok(())
    }

    /// Fetch all site settings (admin only). Returns raw JSON value.
    pub fn list_site_settings(&self) -> Result<Value> {
        let response = self.get("/admin/site_settings.json")?;
        let status = response.status();
        let text = response
            .text()
            .context("reading site settings list response")?;
        if !status.is_success() {
            return Err(http_error("list site settings request", status, &text));
        }
        let value: Value =
            serde_json::from_str(&text).context("parsing site settings list response")?;
        Ok(value)
    }

    /// Fetch all site settings with full metadata (admin only).
    /// Returns one `SiteSettingDetail` per setting, preserving the
    /// `default`, `description`, `category`, and `type` fields.
    pub fn list_site_settings_detailed(&self) -> Result<Vec<SiteSettingDetail>> {
        let raw = self.list_site_settings()?;
        let arr = raw
            .get("site_settings")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow!("site_settings response missing 'site_settings' array"))?;
        let mut out = Vec::with_capacity(arr.len());
        for entry in arr {
            // serde_json::from_value takes ownership, but we can avoid
            // cloning the entire entry by serializing only the borrow and
            // re-parsing — cheaper for large entries. In practice
            // `from_value(entry.clone())` is fine for settings catalogues
            // (each entry is small), but we can avoid the clone by using
            // `serde_json::from_str(&serde_json::to_string(entry)?)` which
            // only borrows. However, that double-serializes. The cleanest
            // fix is to deserialize from a borrowed value via `from_value`
            // on a clone — but we can at least avoid the intermediate
            // allocation by using `from_str` on the borrowed JSON.
            //
            // Actually the simplest improvement: just use from_value
            // on the borrowed entry by taking a reference and letting
            // serde consume it. But `from_value` requires `Value` (owned).
            // The real fix is `serde_json::from_str(&entry.to_string())`
            // which avoids cloning the subtree. For settings (small JSON
            // objects) the difference is negligible, but it's cleaner.
            let detail: SiteSettingDetail =
                serde_json::from_str(&entry.to_string()).with_context(|| {
                    format!(
                        "parsing site setting entry: {}",
                        entry.get("setting").and_then(|v| v.as_str()).unwrap_or("?")
                    )
                })?;
            out.push(detail);
        }
        Ok(out)
    }

    /// Fetch a single site setting by name (admin only).
    /// Returns the value as a string, or an error if not found.
    pub fn fetch_site_setting(&self, setting: &str) -> Result<String> {
        let setting = setting.trim();
        if setting.is_empty() {
            return Err(anyhow!("missing site setting name"));
        }
        // The admin site settings API returns all settings; we filter by name.
        let all = self.list_site_settings()?;
        // Response shape: { "site_settings": [ { "setting": "...", "value": ... }, ... ] }
        // Iterate the borrowed array instead of cloning the entire settings list.
        let arr = all
            .get("site_settings")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow!("site_settings response missing 'site_settings' array"))?;
        for entry in arr {
            let name = entry.get("setting").and_then(|v| v.as_str()).unwrap_or("");
            if name == setting {
                let value = entry.get("value").unwrap_or(&Value::Null);
                let display = match value {
                    Value::String(s) => s.clone(),
                    Value::Null => String::new(),
                    other => other.to_string(),
                };
                return Ok(display);
            }
        }
        Err(anyhow!("setting not found: {}", setting))
    }
}

/// Build the form body for `PUT /admin/site_settings/{name}.json`.
///
/// Discourse expects the new value under a field **named after the setting**
/// (e.g. `title=My+Forum`), not a generic `value=...`. Sending `value=`
/// silently no-ops - and blanks string settings, since the real field is
/// then absent. Regression guard for
/// <https://github.com/koloki-co/dsc/issues/19>.
fn site_setting_form<'a>(setting: &'a str, value: &'a str) -> [(&'a str, &'a str); 1] {
    [(setting, value)]
}

#[cfg(test)]
mod tests {
    use super::site_setting_form;

    #[test]
    fn form_field_is_named_after_the_setting_not_value() {
        // The bug: this used to be `[("value", value)]`, which Discourse
        // ignores - blanking string settings and no-op'ing booleans.
        assert_eq!(
            site_setting_form("title", "My Forum"),
            [("title", "My Forum")]
        );
        assert_eq!(
            site_setting_form("manual_polling_enabled", "true"),
            [("manual_polling_enabled", "true")]
        );
    }
}

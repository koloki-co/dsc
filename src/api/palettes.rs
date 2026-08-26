// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};
use std::collections::BTreeMap;

use super::client::{DiscourseClient, ResponseBody};
use super::error::http_error;

impl DiscourseClient {
    /// List color schemes (palettes) available on the Discourse instance.
    pub fn list_color_schemes(&self) -> Result<Value> {
        let response = self.get("/admin/color_schemes.json")?;
        let status = response.status();
        let text = response
            .text_capped()
            .context("reading color schemes response body")?;
        if !status.is_success() {
            return Err(http_error("color schemes request", status, &text));
        }
        let value: Value = serde_json::from_str(&text).context("parsing color schemes response")?;
        Ok(value)
    }

    /// Fetch a color scheme (palette) by ID.
    pub fn fetch_color_scheme(&self, scheme_id: i64) -> Result<Value> {
        let response = self.list_color_schemes()?;
        color_schemes(&response)?
            .iter()
            .find(|scheme| {
                scheme
                    .get("id")
                    .or_else(|| scheme.get("color_scheme_id"))
                    .and_then(Value::as_i64)
                    == Some(scheme_id)
            })
            .cloned()
            .ok_or_else(|| anyhow!("color scheme not found: {}", scheme_id))
    }

    /// Create a new color scheme (palette).
    pub fn create_color_scheme(
        &self,
        name: &str,
        colors: &BTreeMap<String, String>,
    ) -> Result<i64> {
        let payload = color_scheme_payload(Some(name), colors);
        let response =
            self.send_retrying(|| Ok(self.post("/admin/color_schemes.json")?.json(&payload)))?;
        let status = response.status();
        let text = response
            .text_capped()
            .context("reading color scheme response")?;
        if !status.is_success() {
            return Err(http_error("create color scheme request", status, &text));
        }
        let value: Value =
            serde_json::from_str(&text).context("parsing create color scheme response")?;
        let id = value
            .get("color_scheme")
            .and_then(|v| v.get("id"))
            .or_else(|| value.get("id"))
            .and_then(|v| v.as_i64())
            .ok_or_else(|| anyhow!("missing color scheme id in response"))?;
        Ok(id)
    }

    /// Update an existing color scheme (palette).
    pub fn update_color_scheme(
        &self,
        scheme_id: i64,
        name: Option<&str>,
        colors: &BTreeMap<String, String>,
    ) -> Result<()> {
        let payload = color_scheme_payload(name.filter(|name| !name.trim().is_empty()), colors);
        let path = format!("/admin/color_schemes/{}.json", scheme_id);
        let response = self.send_retrying(|| Ok(self.put(&path)?.json(&payload)))?;
        let status = response.status();
        let text = response
            .text_capped()
            .context("reading color scheme response")?;
        if !status.is_success() {
            return Err(http_error("update color scheme request", status, &text));
        }
        Ok(())
    }
}

pub(crate) fn color_schemes(value: &Value) -> Result<&[Value]> {
    value
        .as_array()
        .or_else(|| value.get("color_schemes").and_then(Value::as_array))
        .map(Vec::as_slice)
        .ok_or_else(|| anyhow!("color schemes response is not an array"))
}

fn color_scheme_payload(name: Option<&str>, colors: &BTreeMap<String, String>) -> Value {
    let colors: Vec<Value> = colors
        .iter()
        .map(|(name, hex)| json!({ "name": name, "hex": hex }))
        .collect();
    let mut color_scheme = serde_json::Map::new();
    if let Some(name) = name {
        color_scheme.insert("name".to_string(), Value::String(name.to_string()));
    }
    color_scheme.insert("colors".to_string(), Value::Array(colors));
    json!({ "color_scheme": color_scheme })
}

#[cfg(test)]
mod tests {
    use super::{color_scheme_payload, color_schemes};
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test]
    fn accepts_current_bare_and_legacy_wrapped_list_shapes() {
        let bare = json!([{ "id": 1 }]);
        let wrapped = json!({ "color_schemes": [{ "id": 2 }] });
        assert_eq!(color_schemes(&bare).unwrap()[0]["id"], 1);
        assert_eq!(color_schemes(&wrapped).unwrap()[0]["id"], 2);
    }

    #[test]
    fn rejects_unknown_list_shape() {
        let response = json!({ "unexpected": [] });
        assert_eq!(
            color_schemes(&response).unwrap_err().to_string(),
            "color schemes response is not an array"
        );
    }

    #[test]
    fn payload_encodes_colors_as_name_hex_rows() {
        let colors = BTreeMap::from([
            ("primary".to_string(), "222222".to_string()),
            ("secondary".to_string(), "FFFFFF".to_string()),
        ]);
        let payload = color_scheme_payload(Some("Test"), &colors);
        assert_eq!(payload["color_scheme"]["name"], "Test");
        assert_eq!(
            payload["color_scheme"]["colors"],
            json!([
                { "name": "primary", "hex": "222222" },
                { "name": "secondary", "hex": "FFFFFF" }
            ])
        );
    }
}

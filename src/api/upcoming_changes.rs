// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! Client for Discourse's Upcoming Changes admin API
//! (`/admin/config/upcoming-changes.json`), list/show plus the explicit enable/disable toggle. See
//! `spec/commands/upcoming-changes-and-setting-upload.md` for the full
//! discovery notes.

use super::client::{DiscourseClient, ResponseBody};
use super::error::http_error;
use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// The `upcoming_change` sub-object attached to an eligible setting.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UpcomingChangeDetail {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub impact: Option<String>,
    #[serde(default)]
    pub impact_type: Option<String>,
    #[serde(default)]
    pub impact_role: Option<String>,
    #[serde(default)]
    pub enabled_for: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// One entry of the Upcoming Changes list, as returned by
/// `GET /admin/config/upcoming-changes.json`. Fields whose exact shape was
/// not captured (`depends_on`, `depends_on_humanized_names`, `groups`) are
/// kept as raw JSON rather than guessed at.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UpcomingChange {
    pub setting: String,
    #[serde(default)]
    pub humanized_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub value: Value,
    #[serde(default)]
    pub upcoming_change: Option<UpcomingChangeDetail>,
    #[serde(default)]
    pub plugin: Option<String>,
    #[serde(default)]
    pub depends_on: Option<Value>,
    #[serde(default)]
    pub depends_on_humanized_names: Option<Value>,
    #[serde(default)]
    pub dependents: Vec<Value>,
    #[serde(default)]
    pub depends_on_met: Option<bool>,
    #[serde(default)]
    pub overriding_defaults: Option<bool>,
    #[serde(default)]
    pub groups: Option<Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Deserialize)]
struct UpcomingChangesEnvelope {
    #[serde(default)]
    upcoming_changes: Vec<UpcomingChange>,
}

const UPCOMING_CHANGES_PATH: &str = "/admin/config/upcoming-changes.json";
const UPCOMING_CHANGES_TOGGLE_PATH: &str = "/admin/config/upcoming-changes/toggle.json";

impl UpcomingChange {
    /// Whether the change is effectively on. Discourse serialises the value
    /// as a JSON boolean; tolerate the `t`/`true` string forms too.
    pub fn is_enabled(&self) -> bool {
        match &self.value {
            Value::Bool(b) => *b,
            Value::String(s) => matches!(s.as_str(), "true" | "t"),
            _ => false,
        }
    }
}

impl DiscourseClient {
    /// List every Upcoming Change the server exposes, in its own stable
    /// setting-name order. The list controller responds only to an XHR
    /// request.
    pub fn list_upcoming_changes(&self) -> Result<Vec<UpcomingChange>> {
        let response = self.get_xhr(UPCOMING_CHANGES_PATH)?;
        let status = response.status();
        let text = response
            .text_capped()
            .context("reading upcoming changes response")?;
        if !status.is_success() {
            return Err(upcoming_changes_http_error(
                "upcoming change list",
                status,
                &text,
            ));
        }
        let envelope: UpcomingChangesEnvelope =
            serde_json::from_str(&text).context("parsing upcoming changes response")?;
        Ok(envelope.upcoming_changes)
    }

    /// Select one Upcoming Change by its exact setting name. There is no
    /// single-item endpoint, so this performs the same list request as
    /// [`DiscourseClient::list_upcoming_changes`] and filters client-side.
    pub fn show_upcoming_change(&self, setting_name: &str) -> Result<UpcomingChange> {
        let changes = self.list_upcoming_changes()?;
        changes
            .into_iter()
            .find(|change| change.setting == setting_name)
            .ok_or_else(|| {
                anyhow!(
                    "no Upcoming Change named '{setting_name}' on this forum — it may not exist \
                     on this Discourse version, may already be a promoted ordinary setting, or \
                     the name may be misspelled (run `dsc upcoming-change list` to see what is \
                     available)"
                )
            })
    }

    /// Send the caller's explicit target state to the toggle endpoint. The
    /// route is named `toggle` but takes an explicit boolean, so this is
    /// idempotent server-side.
    pub fn set_upcoming_change(&self, setting_name: &str, enabled: bool) -> Result<()> {
        let params = [
            ("setting_name", setting_name.to_string()),
            ("enabled", enabled.to_string()),
        ];
        let response = self.send_retrying(|| {
            Ok(self
                .put(UPCOMING_CHANGES_TOGGLE_PATH)?
                .header("X-Requested-With", "XMLHttpRequest")
                .form(&params))
        })?;
        let status = response.status();
        let text = response
            .text_capped()
            .context("reading upcoming change toggle response")?;
        if !status.is_success() {
            return Err(upcoming_changes_http_error(
                "upcoming change toggle",
                status,
                &text,
            ));
        }
        Ok(())
    }
}

fn upcoming_changes_http_error(
    action: &str,
    status: reqwest::StatusCode,
    text: &str,
) -> anyhow::Error {
    if status == reqwest::StatusCode::NOT_FOUND {
        return anyhow!(
            "{}; the Upcoming Changes admin API may be unavailable on this Discourse version",
            http_error(action, status, text)
        );
    }
    http_error(action, status, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_tolerates_unknown_fields_and_missing_optionals() {
        let envelope: UpcomingChangesEnvelope = serde_json::from_str(
            r#"{"upcoming_changes":[{"setting":"enable_generated_llms_txt","value":true,"some_new_field":"x"}]}"#,
        )
        .unwrap();
        assert_eq!(envelope.upcoming_changes.len(), 1);
        let change = &envelope.upcoming_changes[0];
        assert_eq!(change.setting, "enable_generated_llms_txt");
        assert_eq!(change.value, Value::Bool(true));
        assert!(change.humanized_name.is_none());
        assert!(change.upcoming_change.is_none());
        assert!(change.extra.contains_key("some_new_field"));
    }

    #[test]
    fn list_parses_a_representative_entry() {
        let envelope: UpcomingChangesEnvelope = serde_json::from_str(
            r#"{"upcoming_changes":[{
                "setting": "enable_generated_llms_txt",
                "humanized_name": "Enable generated llms txt",
                "description": "Generates a concise /llms.txt when no custom file is uploaded.",
                "value": true,
                "upcoming_change": {
                    "status": "beta",
                    "impact": "feature,all_members",
                    "impact_type": "feature",
                    "impact_role": "all_members",
                    "enabled_for": "everyone"
                },
                "plugin": null,
                "depends_on": null,
                "depends_on_humanized_names": null,
                "dependents": [],
                "depends_on_met": true,
                "overriding_defaults": true,
                "groups": null
            }]}"#,
        )
        .unwrap();
        let change = &envelope.upcoming_changes[0];
        assert_eq!(
            change.humanized_name.as_deref(),
            Some("Enable generated llms txt")
        );
        let detail = change.upcoming_change.as_ref().expect("upcoming_change");
        assert_eq!(detail.status.as_deref(), Some("beta"));
        assert_eq!(detail.enabled_for.as_deref(), Some("everyone"));
        assert_eq!(change.depends_on_met, Some(true));
        assert_eq!(change.overriding_defaults, Some(true));
    }

    #[test]
    fn show_selects_the_exact_named_change() {
        let envelope: UpcomingChangesEnvelope = serde_json::from_str(
            r#"{"upcoming_changes":[
                {"setting":"alpha_feature","value":false},
                {"setting":"beta_feature","value":true}
            ]}"#,
        )
        .unwrap();
        let found = envelope
            .upcoming_changes
            .into_iter()
            .find(|change| change.setting == "beta_feature");
        assert!(found.is_some());
        assert_eq!(found.unwrap().value, Value::Bool(true));
    }

    #[test]
    fn not_found_hint_names_upcoming_changes_availability() {
        let err =
            upcoming_changes_http_error("upcoming change list", reqwest::StatusCode::NOT_FOUND, "");
        let s = err.to_string();
        assert!(
            s.contains("Upcoming Changes"),
            "expected an Upcoming Changes-specific hint: {s}"
        );
    }
}

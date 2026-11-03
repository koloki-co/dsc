// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::api::DiscourseClient;
use crate::cli::ListFormat;
use crate::commands::common::{emit_with, ensure_api_credentials, select_discourse};
use crate::config::Config;
use crate::utils::normalize_baseurl;
use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

const SNAPSHOT_VERSION: u64 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct UserFieldsFile {
    version: u64,
    source: String,
    user_fields: Vec<UserFieldFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct UserFieldFile {
    #[serde(default)]
    id: Option<i64>,
    name: String,
    description: String,
    field_type: String,
    editable: bool,
    requirement: String,
    show_on_profile: bool,
    show_on_signup: bool,
    show_on_user_card: bool,
    searchable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    options: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
struct UserFieldPlan {
    create: Vec<String>,
    unchanged: Vec<String>,
}

pub fn user_field_list(config: &Config, discourse_name: &str, format: ListFormat) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let fields = DiscourseClient::new(discourse)?.list_user_fields()?;
    emit_with(format, &fields, || {
        if fields.is_empty() {
            println!("No custom user fields found.");
        }
        for field in &fields {
            println!(
                "id:{:<4} {:<28} {:<12} {}",
                field["id"].as_i64().unwrap_or_default(),
                field["name"].as_str().unwrap_or("-"),
                field["field_type"].as_str().unwrap_or("-"),
                field["requirement"].as_str().unwrap_or("optional")
            );
        }
        Ok(())
    })
}

pub fn user_field_show(
    config: &Config,
    discourse_name: &str,
    id: i64,
    format: ListFormat,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let field = DiscourseClient::new(discourse)?.get_user_field(id)?;
    emit_with(format, &field, || {
        println!("id:          {}", field["id"]);
        println!("name:        {}", field["name"]);
        println!("description: {}", field["description"]);
        println!("field_type:  {}", field["field_type"]);
        println!("requirement: {}", field["requirement"]);
        if let Some(options) = field["options"].as_array() {
            println!("options:");
            for option in options {
                println!("  - {}", option.as_str().unwrap_or_default());
            }
        }
        Ok(())
    })
}

pub fn user_field_pull(
    config: &Config,
    discourse_name: &str,
    path: &Path,
    force: bool,
) -> Result<()> {
    if path.exists() && !force {
        bail!(
            "{} already exists; use --force to overwrite",
            path.display()
        );
    }
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let fields = DiscourseClient::new(discourse)?.list_user_fields()?;
    let file = UserFieldsFile {
        version: SNAPSHOT_VERSION,
        source: normalize_baseurl(&discourse.baseurl),
        user_fields: fields.iter().map(field_from_value).collect::<Result<_>>()?,
    };
    let content = if path.extension().is_some_and(|ext| ext == "json") {
        serde_json::to_string_pretty(&file)?
    } else {
        serde_yaml::to_string(&file)?
    };
    fs::write(path, content).with_context(|| format!("writing {}", path.display()))?;
    println!(
        "Wrote {} user field definition(s) to {}",
        file.user_fields.len(),
        path.display()
    );
    Ok(())
}

pub fn user_field_diff(
    config: &Config,
    discourse_name: &str,
    path: &Path,
    format: ListFormat,
) -> Result<()> {
    let (plan, _) = plan(config, discourse_name, path)?;
    emit_with(format, &plan, || print_plan(&plan))
}

pub fn user_field_push(
    config: &Config,
    discourse_name: &str,
    path: &Path,
    dry_run: bool,
) -> Result<()> {
    let (plan, file) = plan(config, discourse_name, path)?;
    if dry_run {
        print_plan(&plan)?;
        println!("[dry-run] No changes applied.");
        return Ok(());
    }
    let discourse = select_discourse(config, Some(discourse_name))?;
    let client = DiscourseClient::new(discourse)?;
    for field in &file.user_fields {
        let created = client.create_user_field(&field_to_value(field))?;
        if created["name"].as_str() != Some(&field.name) {
            bail!(
                "post-write verification failed for user field '{}'",
                field.name
            );
        }
        println!("Created user field '{}' (id {})", field.name, created["id"]);
    }
    Ok(())
}

fn plan(
    config: &Config,
    discourse_name: &str,
    path: &Path,
) -> Result<(UserFieldPlan, UserFieldsFile)> {
    let content =
        fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let file: UserFieldsFile = if path.extension().is_some_and(|ext| ext == "json") {
        serde_json::from_str(&content)?
    } else {
        serde_yaml::from_str(&content)?
    };
    if file.version != SNAPSHOT_VERSION {
        bail!("unsupported user field snapshot version {}", file.version);
    }
    let discourse = select_discourse(config, Some(discourse_name))?;
    let source = normalize_baseurl(&discourse.baseurl);
    if normalize_baseurl(&file.source) != source {
        bail!(
            "snapshot source '{}' does not match target '{}'; remove IDs and set source deliberately before cross-forum creation",
            file.source,
            source
        );
    }
    validate(&file)?;
    ensure_api_credentials(discourse)?;
    let live = DiscourseClient::new(discourse)?.list_user_fields()?;
    let live_names: BTreeSet<&str> = live
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    let mut create = Vec::new();
    for field in &file.user_fields {
        if let Some(id) = field.id {
            bail!(
                "user field '{}' has id {}; updates are not enabled yet because option replacement is destructive",
                field.name,
                id
            );
        }
        if live_names.contains(field.name.as_str()) {
            bail!(
                "user field '{}' already exists; pull the target before reconciling it",
                field.name
            );
        }
        create.push(field.name.clone());
    }
    Ok((
        UserFieldPlan {
            create,
            unchanged: Vec::new(),
        },
        file,
    ))
}

fn validate(file: &UserFieldsFile) -> Result<()> {
    let mut names = BTreeSet::new();
    for field in &file.user_fields {
        if field.name.trim().is_empty() || field.description.trim().is_empty() {
            bail!("user field names and descriptions must not be blank");
        }
        if !matches!(
            field.field_type.as_str(),
            "text" | "textarea" | "date" | "confirm" | "dropdown" | "multiselect"
        ) {
            bail!(
                "user field '{}' has unsupported field_type '{}'",
                field.name,
                field.field_type
            );
        }
        if !matches!(
            field.requirement.as_str(),
            "optional" | "on_signup" | "for_all_users"
        ) {
            bail!(
                "user field '{}' has invalid requirement '{}'",
                field.name,
                field.requirement
            );
        }
        if !names.insert(field.name.as_str()) {
            bail!("duplicate user field name '{}'", field.name);
        }
        if matches!(field.field_type.as_str(), "dropdown" | "multiselect") {
            let options = field
                .options
                .as_ref()
                .ok_or_else(|| anyhow!("user field '{}' requires options", field.name))?;
            if options.is_empty() {
                bail!("user field '{}' requires at least one option", field.name);
            }
            let mut values = BTreeSet::new();
            for option in options {
                if option.trim().is_empty() || !values.insert(option) {
                    bail!("user field '{}' has blank or duplicate options", field.name);
                }
            }
        } else if field.options.is_some() {
            bail!(
                "user field '{}' must not define options for type '{}'",
                field.name,
                field.field_type
            );
        }
    }
    Ok(())
}

fn field_from_value(value: &Value) -> Result<UserFieldFile> {
    let string = |key: &str| {
        value[key]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| anyhow!("user field is missing {key}"))
    };
    Ok(UserFieldFile {
        id: value["id"].as_i64(),
        name: string("name")?,
        description: string("description")?,
        field_type: string("field_type")?,
        editable: value["editable"]
            .as_bool()
            .ok_or_else(|| anyhow!("user field is missing editable"))?,
        requirement: string("requirement")?,
        show_on_profile: value["show_on_profile"]
            .as_bool()
            .ok_or_else(|| anyhow!("user field is missing show_on_profile"))?,
        show_on_signup: value["show_on_signup"]
            .as_bool()
            .ok_or_else(|| anyhow!("user field is missing show_on_signup"))?,
        show_on_user_card: value["show_on_user_card"]
            .as_bool()
            .ok_or_else(|| anyhow!("user field is missing show_on_user_card"))?,
        searchable: value["searchable"]
            .as_bool()
            .ok_or_else(|| anyhow!("user field is missing searchable"))?,
        options: value["options"].as_array().map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        }),
    })
}

fn field_to_value(field: &UserFieldFile) -> Value {
    let mut value = Map::new();
    value.insert("name".into(), field.name.clone().into());
    value.insert("description".into(), field.description.clone().into());
    value.insert("field_type".into(), field.field_type.clone().into());
    value.insert("editable".into(), field.editable.into());
    value.insert("requirement".into(), field.requirement.clone().into());
    value.insert("show_on_profile".into(), field.show_on_profile.into());
    value.insert("show_on_signup".into(), field.show_on_signup.into());
    value.insert("show_on_user_card".into(), field.show_on_user_card.into());
    value.insert("searchable".into(), field.searchable.into());
    if let Some(options) = &field.options {
        value.insert(
            "options".into(),
            Value::Array(options.iter().cloned().map(Value::String).collect()),
        );
    }
    Value::Object(value)
}

fn print_plan(plan: &UserFieldPlan) -> Result<()> {
    for name in &plan.create {
        println!("+ create: {name}");
    }
    if plan.create.is_empty() {
        println!("(no changes)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_field(name: &str) -> UserFieldFile {
        UserFieldFile {
            id: None,
            name: name.into(),
            description: "A description".into(),
            field_type: "text".into(),
            editable: true,
            requirement: "optional".into(),
            show_on_profile: false,
            show_on_signup: false,
            show_on_user_card: false,
            searchable: false,
            options: None,
        }
    }

    fn file(fields: Vec<UserFieldFile>) -> UserFieldsFile {
        UserFieldsFile {
            version: SNAPSHOT_VERSION,
            source: "https://forum.example".into(),
            user_fields: fields,
        }
    }

    #[test]
    fn validate_rejects_duplicate_names() {
        let error = validate(&file(vec![
            text_field("Department"),
            text_field("Department"),
        ]))
        .expect_err("duplicate names must be rejected");

        assert!(error.to_string().contains("duplicate user field name"));
    }

    #[test]
    fn validate_requires_options_for_select_fields() {
        let mut field = text_field("Role");
        field.field_type = "dropdown".into();

        let error = validate(&file(vec![field])).expect_err("dropdown needs options");

        assert!(error.to_string().contains("requires options"));
    }

    #[test]
    fn validate_rejects_empty_options_for_select_fields() {
        let mut field = text_field("Role");
        field.field_type = "dropdown".into();
        field.options = Some(Vec::new());

        let error = validate(&file(vec![field])).expect_err("dropdown needs an option");

        assert!(error.to_string().contains("requires at least one option"));
    }

    #[test]
    fn create_payload_omits_snapshot_id() {
        let mut field = text_field("Department");
        field.id = Some(42);

        let value = field_to_value(&field);

        assert!(value.get("id").is_none());
        assert_eq!(value["name"], "Department");
    }
}

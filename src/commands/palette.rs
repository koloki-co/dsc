// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::api::{DiscourseClient, color_schemes};
use crate::cli::ListFormat;
use crate::commands::common::{ensure_api_credentials, select_discourse};
use crate::config::Config;
use crate::utils::{atomic_write, normalize_baseurl};

#[derive(Debug, Serialize, Deserialize)]
struct PaletteFile {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<i64>,
    name: String,
    colors: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pulled: Option<PaletteBaseline>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PaletteBaseline {
    name: String,
    colors: BTreeMap<String, String>,
}

#[derive(Debug, Serialize)]
struct PaletteListEntry {
    id: i64,
    name: String,
}

pub fn palette_list(
    config: &Config,
    discourse_name: &str,
    format: ListFormat,
    verbose: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let response = client.list_color_schemes()?;
    let entries: Vec<PaletteListEntry> = color_schemes(&response)?
        .iter()
        .map(|scheme| {
            let id = scheme
                .get("id")
                .or_else(|| scheme.get("color_scheme_id"))
                .and_then(Value::as_i64)
                .ok_or_else(|| anyhow!("color scheme is missing a signed integer id"))?;
            let name = scheme
                .get("name")
                .or_else(|| scheme.get("color_scheme_name"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            Ok(PaletteListEntry { id, name })
        })
        .collect::<Result<_>>()?;

    match format {
        ListFormat::Text => {
            if entries.is_empty() && !verbose {
                println!("No palettes found.");
                return Ok(());
            }
            for entry in entries {
                println!("{} - {}", entry.id, entry.name);
            }
        }
        ListFormat::Json => {
            let raw = serde_json::to_string_pretty(&entries)?;
            println!("{}", raw);
        }
        ListFormat::Yaml => {
            let raw = serde_yaml::to_string(&entries)?;
            println!("{}", raw);
        }
    }
    Ok(())
}

pub fn palette_pull(
    config: &Config,
    discourse_name: &str,
    palette_id: i64,
    local_path: Option<&Path>,
    force: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let response = client.fetch_color_scheme(palette_id)?;
    let palette = palette_from_response(&response, palette_id)?;

    let path = match local_path {
        Some(path) => path.to_path_buf(),
        None => {
            let filename = format!("palette-{}.json", palette_id);
            std::env::current_dir()?.join(filename)
        }
    };
    write_palette_file(&path, &palette, force)?;
    println!("{}", path.display());
    Ok(())
}

pub fn palette_push(
    config: &Config,
    discourse_name: &str,
    local_path: &Path,
    palette_id: Option<i64>,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let palette = read_palette_file(local_path)?;

    if palette.colors.is_empty() {
        return Err(anyhow!("palette file contains no colors"));
    }
    if palette.name.trim().is_empty() {
        return Err(anyhow!("palette file contains an empty name"));
    }

    let target_id = palette_id.or(palette.id);
    if let Some(target_id) = target_id {
        ensure_mutable_palette_id(target_id)?;
        let baseline = palette.pulled.as_ref().ok_or_else(|| {
            anyhow!(
                "palette update requires a pulled baseline; run `dsc theme palette pull` again before editing"
            )
        })?;
        if let Some(source_id) = palette.id
            && source_id != target_id
        {
            return Err(anyhow!(
                "palette file was pulled from id {}; pull palette {target_id} before updating it",
                source_id
            ));
        }
        let current = palette_from_response(&client.fetch_color_scheme(target_id)?, target_id)?;
        let mut changed_colors = changed_colors(&palette.colors, &baseline.colors);
        ensure_no_palette_conflicts(&changed_colors, baseline, &current)?;
        changed_colors.retain(|name, desired| current.colors.get(name) != Some(desired));
        let locally_renamed = palette.name != baseline.name;
        if locally_renamed && current.name != baseline.name && current.name != palette.name {
            return Err(anyhow!(
                "palette name changed remotely from '{}' to '{}'; pull again before pushing",
                baseline.name,
                current.name
            ));
        }
        let changed_name =
            (locally_renamed && current.name != palette.name).then_some(palette.name.as_str());
        if dry_run {
            print_update_plan(
                discourse_name,
                local_path,
                target_id,
                &current,
                changed_name,
                &changed_colors,
            );
            return Ok(());
        }
        if changed_name.is_some() || !changed_colors.is_empty() {
            client.update_color_scheme(target_id, changed_name, &changed_colors)?;
        }
        let refreshed = palette_from_response(&client.fetch_color_scheme(target_id)?, target_id)?;
        write_palette_file(local_path, &refreshed, true)?;
        let url = format!(
            "{}/admin/customize/colors/{}",
            normalize_baseurl(&discourse.baseurl),
            target_id
        );
        println!("{}", url);
    } else {
        if dry_run {
            print_create_plan(discourse_name, local_path, &palette);
            return Ok(());
        }
        let new_id = client.create_color_scheme(&palette.name, &palette.colors)?;
        let mut created_with_id = palette;
        created_with_id.id = Some(new_id);
        write_palette_file(local_path, &created_with_id, true)?;
        let created = palette_from_response(&client.fetch_color_scheme(new_id)?, new_id)?;
        write_palette_file(local_path, &created, true)?;
        let url = format!(
            "{}/admin/customize/colors/{}",
            normalize_baseurl(&discourse.baseurl),
            new_id
        );
        println!("{}", url);
    }

    Ok(())
}

/// Print the full dry-run sequence for an existing palette. The initial GET has
/// already happened while checking conflicts; it is included so the planned
/// live request sequence and local snapshot refresh are explicit.
fn print_update_plan(
    discourse_name: &str,
    local_path: &Path,
    target_id: i64,
    current: &PaletteFile,
    changed_name: Option<&str>,
    changed_colors: &BTreeMap<String, String>,
) {
    println!(
        "[dry-run] {}: palette push plan for {} ({:?}) from {}:",
        discourse_name,
        target_id,
        current.name,
        local_path.display()
    );
    println!(
        "  GET /admin/color_schemes.json (read current palette {})",
        target_id
    );

    let mut changes = 0;
    if let Some(name) = changed_name {
        println!("  ~ name: {:?} -> {:?}", current.name, name);
        changes += 1;
    }
    for (name, value) in changed_colors {
        let current_value = current
            .colors
            .get(name)
            .map(String::as_str)
            .unwrap_or("<unset>");
        println!("  ~ {}: {:?} -> {:?}", name, current_value, value);
        changes += 1;
    }
    if changes == 0 {
        println!("  = unchanged: palette {}", target_id);
    } else {
        println!(
            "  PUT /admin/color_schemes/{}.json ({} change{})",
            target_id,
            changes,
            if changes == 1 { "" } else { "s" }
        );
    }
    println!("  GET /admin/color_schemes.json (refresh palette snapshot)");
    println!(
        "  write refreshed palette snapshot to {}",
        local_path.display()
    );
    println!("[dry-run] No changes applied.");
}

/// Print the complete dry-run sequence for a new palette, including every
/// color that would be sent and the local snapshot created after Discourse
/// assigns an ID.
fn print_create_plan(discourse_name: &str, local_path: &Path, palette: &PaletteFile) {
    println!(
        "[dry-run] {}: palette create plan for {:?} from {}:",
        discourse_name,
        palette.name,
        local_path.display()
    );
    println!(
        "  + create: {:?} ({} color{})",
        palette.name,
        palette.colors.len(),
        if palette.colors.len() == 1 { "" } else { "s" }
    );
    for (name, value) in &palette.colors {
        println!("    + {}: {:?}", name, value);
    }
    println!("  POST /admin/color_schemes.json");
    println!(
        "  write palette snapshot with assigned ID to {}",
        local_path.display()
    );
    println!("  GET /admin/color_schemes.json (fetch created palette and assigned ID)");
    println!(
        "  write refreshed palette snapshot to {}",
        local_path.display()
    );
    println!("[dry-run] No changes applied.");
}

fn palette_from_response(response: &Value, fallback_id: i64) -> Result<PaletteFile> {
    let scheme = response.get("color_scheme").unwrap_or(response);
    let id = scheme
        .get("id")
        .or_else(|| scheme.get("color_scheme_id"))
        .and_then(Value::as_i64)
        .or_else(|| response.get("id").and_then(Value::as_i64))
        .unwrap_or(fallback_id);
    let name = scheme
        .get("name")
        .or_else(|| scheme.get("color_scheme_name"))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("palette is missing a name"))?
        .to_string();
    let colors_value = scheme
        .get("colors")
        .or_else(|| response.get("colors"))
        .unwrap_or(&Value::Null);
    let colors = colors_from_value(colors_value)?;
    if colors.is_empty() {
        return Err(anyhow!("palette is missing color values"));
    }
    let pulled = PaletteBaseline {
        name: name.clone(),
        colors: colors.clone(),
    };
    Ok(PaletteFile {
        id: Some(id),
        name,
        colors,
        pulled: Some(pulled),
    })
}

fn colors_from_value(value: &Value) -> Result<BTreeMap<String, String>> {
    match value {
        Value::Object(map) => map
            .iter()
            .map(|(key, value)| {
                let value = value
                    .as_str()
                    .ok_or_else(|| anyhow!("palette color '{key}' is not a string"))?;
                Ok((key.clone(), value.to_string()))
            })
            .collect(),
        Value::Array(items) => {
            let mut out = BTreeMap::new();
            for item in items {
                let name = item
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow!("palette color row is missing a name"))?;
                let hex = item
                    .get("hex")
                    .and_then(Value::as_str)
                    .or_else(|| item.get("value").and_then(Value::as_str))
                    .ok_or_else(|| anyhow!("palette color '{name}' is missing a hex value"))?;
                out.insert(name.to_string(), hex.to_string());
            }
            Ok(out)
        }
        _ => Err(anyhow!("palette colors are not an object or array")),
    }
}

fn changed_colors(
    desired: &BTreeMap<String, String>,
    current: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    desired
        .iter()
        .filter(|(name, value)| current.get(*name) != Some(*value))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

fn ensure_no_palette_conflicts(
    changed: &BTreeMap<String, String>,
    baseline: &PaletteBaseline,
    current: &PaletteFile,
) -> Result<()> {
    for (name, desired) in changed {
        let pulled = baseline.colors.get(name);
        let remote = current.colors.get(name);
        if remote != pulled && remote != Some(desired) {
            return Err(anyhow!(
                "palette color '{name}' changed remotely; pull again before pushing"
            ));
        }
    }
    Ok(())
}

fn ensure_mutable_palette_id(palette_id: i64) -> Result<()> {
    if palette_id < 0 {
        return Err(anyhow!(
            "built-in palette {palette_id} cannot be updated; create a custom palette instead"
        ));
    }
    Ok(())
}

fn read_palette_file(path: &Path) -> Result<PaletteFile> {
    let raw = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    if is_yaml(path) {
        let palette: PaletteFile = serde_yaml::from_str(&raw).context("parsing palette yaml")?;
        return Ok(palette);
    }
    let palette: PaletteFile = serde_json::from_str(&raw).context("parsing palette json")?;
    Ok(palette)
}

fn write_palette_file(path: &Path, palette: &PaletteFile, overwrite: bool) -> Result<()> {
    let content = if is_yaml(path) {
        serde_yaml::to_string(palette).context("serializing palette yaml")?
    } else {
        serde_json::to_string_pretty(palette).context("serializing palette json")?
    };
    atomic_write(path, content, overwrite)
}

fn is_yaml(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|s| s.to_str()),
        Some("yml") | Some("yaml")
    )
}

#[cfg(test)]
mod tests {
    use super::{
        PaletteBaseline, PaletteFile, changed_colors, colors_from_value, ensure_mutable_palette_id,
        ensure_no_palette_conflicts, palette_from_response,
    };
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test]
    fn parses_negative_builtin_palette_id() {
        let response = json!({
            "id": -2,
            "name": "Dark",
            "colors": [{ "name": "primary", "hex": "FFFFFF" }]
        });
        let palette = palette_from_response(&response, -2).unwrap();
        assert_eq!(palette.id, Some(-2));
        assert_eq!(palette.colors["primary"], "FFFFFF");
        assert_eq!(palette.pulled.unwrap().colors["primary"], "FFFFFF");
    }

    #[test]
    fn unchanged_resolved_colors_are_not_sent_as_overrides() {
        let pulled = BTreeMap::from([
            ("primary".to_string(), "222222".to_string()),
            ("secondary".to_string(), "FFFFFF".to_string()),
            ("hover".to_string(), "444444".to_string()),
        ]);
        let desired = BTreeMap::from([
            ("primary".to_string(), "111111".to_string()),
            ("secondary".to_string(), "FFFFFF".to_string()),
            ("hover".to_string(), "444444".to_string()),
        ]);
        assert_eq!(
            changed_colors(&desired, &pulled),
            BTreeMap::from([("primary".to_string(), "111111".to_string())])
        );

        let refreshed = BTreeMap::from([
            ("primary".to_string(), "111111".to_string()),
            ("secondary".to_string(), "FFFFFF".to_string()),
            ("hover".to_string(), "333333".to_string()),
        ]);
        assert!(changed_colors(&refreshed, &refreshed).is_empty());
    }

    #[test]
    fn remote_palette_edits_are_not_overwritten() {
        let baseline = PaletteBaseline {
            name: "Brand".to_string(),
            colors: BTreeMap::from([("primary".to_string(), "222222".to_string())]),
        };
        let current = PaletteFile {
            id: Some(1),
            name: "Brand".to_string(),
            colors: BTreeMap::from([("primary".to_string(), "333333".to_string())]),
            pulled: Some(PaletteBaseline {
                name: "Brand".to_string(),
                colors: BTreeMap::from([("primary".to_string(), "333333".to_string())]),
            }),
        };
        let changed = BTreeMap::from([("primary".to_string(), "111111".to_string())]);
        assert!(ensure_no_palette_conflicts(&changed, &baseline, &current).is_err());
    }

    #[test]
    fn malformed_color_rows_fail_closed() {
        let error = colors_from_value(&json!([{ "name": "primary" }])).unwrap_err();
        assert_eq!(
            error.to_string(),
            "palette color 'primary' is missing a hex value"
        );
    }

    #[test]
    fn built_in_palettes_cannot_be_updated() {
        assert!(ensure_mutable_palette_id(1).is_ok());
        assert!(
            ensure_mutable_palette_id(-1)
                .unwrap_err()
                .to_string()
                .contains("cannot be updated")
        );
    }
}

// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::api::DiscourseClient;
use crate::cli::ListFormat;
use crate::commands::common::{ensure_api_credentials, select_discourse};
use crate::config::Config;
use crate::utils::{atomic_write, normalize_baseurl};

#[derive(Debug, Serialize, Deserialize)]
struct PaletteFile {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<u64>,
    name: String,
    colors: BTreeMap<String, String>,
}

#[derive(Debug, Serialize)]
struct PaletteListEntry {
    id: u64,
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
    let schemes = response
        .get("color_schemes")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let entries: Vec<PaletteListEntry> = schemes
        .into_iter()
        .map(|scheme| {
            let id = scheme
                .get("id")
                .or_else(|| scheme.get("color_scheme_id"))
                .and_then(|v| v.as_u64())
                .unwrap_or_default();
            let name = scheme
                .get("name")
                .or_else(|| scheme.get("color_scheme_name"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            PaletteListEntry { id, name }
        })
        .collect();

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
    palette_id: u64,
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
    palette_id: Option<u64>,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let mut palette = read_palette_file(local_path)?;

    if palette.colors.is_empty() {
        return Err(anyhow!("palette file contains no colors"));
    }

    let target_id = palette_id.or(palette.id);
    if let Some(target_id) = target_id {
        client.update_color_scheme(target_id, Some(&palette.name), &palette.colors)?;
        let url = format!(
            "{}/admin/customize/colors/{}",
            normalize_baseurl(&discourse.baseurl),
            target_id
        );
        println!("{}", url);
    } else {
        if palette.name.trim().is_empty() {
            return Err(anyhow!("missing palette name for palette create"));
        }
        let new_id = client.create_color_scheme(&palette.name, &palette.colors)?;
        palette.id = Some(new_id);
        write_palette_file(local_path, &palette, true)?;
        let url = format!(
            "{}/admin/customize/colors/{}",
            normalize_baseurl(&discourse.baseurl),
            new_id
        );
        println!("{}", url);
    }

    Ok(())
}

fn palette_from_response(response: &Value, fallback_id: u64) -> Result<PaletteFile> {
    let scheme = response.get("color_scheme").unwrap_or(response);
    let id = scheme
        .get("id")
        .or_else(|| scheme.get("color_scheme_id"))
        .and_then(|v| v.as_u64())
        .or_else(|| response.get("id").and_then(|v| v.as_u64()))
        .unwrap_or(fallback_id);
    let name = scheme
        .get("name")
        .or_else(|| scheme.get("color_scheme_name"))
        .and_then(|v| v.as_str())
        .unwrap_or("palette")
        .to_string();
    let colors_value = scheme
        .get("colors")
        .or_else(|| response.get("colors"))
        .unwrap_or(&Value::Null);
    let colors = colors_from_value(colors_value);
    if colors.is_empty() {
        return Err(anyhow!("palette is missing color values"));
    }
    Ok(PaletteFile {
        id: Some(id),
        name,
        colors,
    })
}

fn colors_from_value(value: &Value) -> BTreeMap<String, String> {
    match value {
        Value::Object(map) => map
            .iter()
            .filter_map(|(key, value)| value.as_str().map(|val| (key.clone(), val.to_string())))
            .collect(),
        Value::Array(items) => {
            let mut out = BTreeMap::new();
            for item in items {
                if let Some(name) = item.get("name").and_then(|v| v.as_str())
                    && let Some(hex) = item
                        .get("hex")
                        .and_then(|v| v.as_str())
                        .or_else(|| item.get("value").and_then(|v| v.as_str()))
                {
                    out.insert(name.to_string(), hex.to_string());
                }
            }
            out
        }
        _ => BTreeMap::new(),
    }
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

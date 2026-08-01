// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::api::{
    DiscourseClient, ExplorerParamInfo, ExplorerQueryDetails, ExplorerQuerySummary,
    ExplorerRunResult,
};
use crate::cli::ListFormat;
use crate::commands::common::{ensure_api_credentials, select_discourse};
use crate::config::Config;
use crate::utils::atomic_write_private;
use anyhow::{Context, Result, anyhow};
use serde_json::{Map, Value};
use std::fs;
use std::path::Path;

/// Options for listing saved Data Explorer queries.
pub struct ExplorerListOptions<'a> {
    pub filter: Option<&'a str>,
    pub order: Option<&'a str>,
    pub ascending: bool,
    pub format: ListFormat,
}

/// Options for executing a saved Data Explorer query.
pub struct ExplorerRunOptions<'a> {
    pub params: Option<&'a str>,
    pub params_file: Option<&'a Path>,
    pub csv: Option<&'a Path>,
    pub explain: bool,
    pub limit: Option<u32>,
    pub format: ListFormat,
}

/// List all accessible saved Data Explorer queries.
pub fn explorer_list(
    config: &Config,
    discourse_name: &str,
    options: ExplorerListOptions<'_>,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let catalogue =
        client.list_explorer_queries(options.filter, options.order, options.ascending)?;

    match options.format {
        ListFormat::Text => print_query_list(&catalogue.queries),
        ListFormat::Json => println!("{}", serde_json::to_string_pretty(&catalogue)?),
        ListFormat::Yaml => println!("{}", serde_yaml::to_string(&catalogue)?),
    }
    Ok(())
}

/// Show or export one saved Data Explorer query definition.
pub fn explorer_show(
    config: &Config,
    discourse_name: &str,
    query_id: i64,
    export: Option<&Path>,
    format: ListFormat,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    if let Some(path) = export {
        let bytes = client.export_explorer_query(query_id)?;
        atomic_write_private(path, bytes, false)?;
        eprintln!("Wrote Data Explorer query export to {}", path.display());
        return Ok(());
    }

    let query = client.show_explorer_query(query_id)?;
    match format {
        ListFormat::Text => print_query_details(&query),
        ListFormat::Json => println!("{}", serde_json::to_string_pretty(&query)?),
        ListFormat::Yaml => println!("{}", serde_yaml::to_string(&query)?),
    }
    Ok(())
}

/// Run one saved Data Explorer query, rendering JSON/YAML/text or writing CSV.
pub fn explorer_run(
    config: &Config,
    discourse_name: &str,
    query_id: i64,
    options: ExplorerRunOptions<'_>,
    dry_run: bool,
) -> Result<()> {
    let params = load_params(options.params, options.params_file)?;
    if options.limit == Some(0) {
        return Err(anyhow!("--limit must be greater than zero"));
    }

    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    // Running a saved query executes read-only SQL, but Discourse still
    // records `last_run_at` on the query and charges the API rate limit, so
    // `--dry-run` must describe the request rather than send it.
    if dry_run {
        let destination = match options.csv {
            Some(path) => format!("CSV file {}", path.display()),
            None => "stdout".to_string(),
        };
        println!(
            "[dry-run] {}: would run Data Explorer query {} ({} parameter{}) writing to {}",
            discourse.name,
            query_id,
            params.len(),
            if params.len() == 1 { "" } else { "s" },
            destination
        );
        if !params.is_empty() {
            println!("  params: {}", serde_json::to_string(&params)?);
        }
        if let Some(limit) = options.limit {
            println!("  limit: {}", limit);
        }
        if options.explain {
            println!("  explain: requested");
        }
        return Ok(());
    }

    if let Some(path) = options.csv {
        let bytes = client.download_explorer_query_csv(query_id, &params, options.limit)?;
        atomic_write_private(path, bytes, false)?;
        eprintln!("Wrote Data Explorer CSV result to {}", path.display());
        return Ok(());
    }

    let result = client.run_explorer_query(query_id, &params, options.explain, options.limit)?;
    match options.format {
        ListFormat::Text => print_run_result(&result),
        ListFormat::Json => println!("{}", serde_json::to_string_pretty(&result)?),
        ListFormat::Yaml => println!("{}", serde_yaml::to_string(&result)?),
    }
    Ok(())
}

fn load_params(inline: Option<&str>, file: Option<&Path>) -> Result<Map<String, Value>> {
    let value = match (inline, file) {
        (Some(_), Some(_)) => {
            return Err(anyhow!(
                "use exactly one of --params or --params-file, not both"
            ));
        }
        (Some(raw), None) => {
            serde_json::from_str(raw).context("parsing --params as a JSON object")?
        }
        (None, Some(path)) => parse_params_file(path)?,
        (None, None) => Value::Object(Map::new()),
    };
    value
        .as_object()
        .cloned()
        .ok_or_else(|| anyhow!("Data Explorer parameters must be an object"))
}

fn parse_params_file(path: &Path) -> Result<Value> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("reading parameter file {}", path.display()))?;
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("json") => serde_json::from_str(&raw)
            .with_context(|| format!("parsing {} as JSON", path.display())),
        Some("yaml" | "yml") => serde_yaml::from_str(&raw)
            .with_context(|| format!("parsing {} as YAML", path.display())),
        _ => serde_json::from_str(&raw).or_else(|json_error| {
            serde_yaml::from_str(&raw).with_context(|| {
                format!("parsing {} as JSON ({json_error}) or YAML", path.display())
            })
        }),
    }
}

fn print_query_list(queries: &[ExplorerQuerySummary]) {
    if queries.is_empty() {
        println!("No Data Explorer queries found.");
        return;
    }
    for query in queries {
        let owner = query.username.as_deref().unwrap_or("-");
        let last_run = query.last_run_at.as_deref().unwrap_or("never");
        let groups = if query.group_ids.is_empty() {
            "-".to_string()
        } else {
            query
                .group_ids
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(",")
        };
        let default = if query.is_default { " default" } else { "" };
        println!(
            "{:>5}  {}{}  owner:{}  last:{}  groups:{}",
            query.id, query.name, default, owner, last_run, groups
        );
        if let Some(description) = query
            .description
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            println!("       {}", one_line(description));
        }
    }
}

fn print_query_details(query: &ExplorerQueryDetails) {
    println!("id:          {}", query.id);
    println!("name:        {}", query.name);
    println!(
        "description: {}",
        query.description.as_deref().unwrap_or("-")
    );
    println!("owner:       {}", query.username.as_deref().unwrap_or("-"));
    println!("default:     {}", query.is_default);
    println!("hidden:      {}", query.hidden);
    println!(
        "groups:      {}",
        if query.group_ids.is_empty() {
            "-".to_string()
        } else {
            query
                .group_ids
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        }
    );
    println!(
        "last run:    {}",
        query.last_run_at.as_deref().unwrap_or("never")
    );
    println!(
        "created:     {}",
        query.created_at.as_deref().unwrap_or("-")
    );
    print_param_info(&query.param_info);
    println!("\nsql:");
    println!("{}", query.sql.as_deref().unwrap_or("(not returned)"));
    if let Some(cached) = &query.cached_result {
        println!("\ncached result:");
        print_run_result(cached);
    }
}

fn print_param_info(params: &[ExplorerParamInfo]) {
    if params.is_empty() {
        println!("parameters:  none");
        return;
    }
    println!("parameters:");
    for param in params {
        let mut attributes = Vec::new();
        if let Some(default) = &param.default {
            attributes.push(format!("default={}", display_value(default)));
        }
        if param.nullable {
            attributes.push("nullable".to_string());
        }
        if param.internal {
            attributes.push("internal".to_string());
        }
        let suffix = if attributes.is_empty() {
            String::new()
        } else {
            format!(" ({})", attributes.join(", "))
        };
        println!("  {}: {}{}", param.identifier, param.param_type, suffix);
    }
}

fn print_run_result(result: &ExplorerRunResult) {
    let column_count = result
        .rows
        .iter()
        .map(Vec::len)
        .max()
        .unwrap_or(0)
        .max(result.columns.len());
    if column_count == 0 {
        println!("No rows returned.");
    } else {
        let headers: Vec<String> = (0..column_count)
            .map(|index| {
                result
                    .columns
                    .get(index)
                    .cloned()
                    .unwrap_or_else(|| format!("column_{}", index + 1))
            })
            .collect();
        let rendered_rows: Vec<Vec<String>> = result
            .rows
            .iter()
            .map(|row| {
                (0..column_count)
                    .map(|index| row.get(index).map(display_value).unwrap_or_default())
                    .collect()
            })
            .collect();
        let widths: Vec<usize> = (0..column_count)
            .map(|index| {
                rendered_rows
                    .iter()
                    .map(|row| row[index].chars().count())
                    .chain(std::iter::once(headers[index].chars().count()))
                    .max()
                    .unwrap_or(0)
            })
            .collect();
        print_table_row(&headers, &widths);
        println!(
            "{}",
            widths
                .iter()
                .map(|width| "-".repeat(*width))
                .collect::<Vec<_>>()
                .join("  ")
        );
        for row in &rendered_rows {
            print_table_row(row, &widths);
        }
    }
    let duration = result
        .duration
        .map(|milliseconds| format!(", {milliseconds:.1} ms"))
        .unwrap_or_default();
    eprintln!("{} row(s){duration}", result.rows.len());
    if let Some(explain) = &result.explain {
        println!("\nexplain:");
        println!("{explain}");
    }
}

fn print_table_row(cells: &[String], widths: &[usize]) {
    println!(
        "{}",
        cells
            .iter()
            .enumerate()
            .map(|(index, cell)| format!("{cell:<width$}", width = widths[index]))
            .collect::<Vec<_>>()
            .join("  ")
    );
}

fn display_value(value: &Value) -> String {
    one_line(match value {
        Value::String(value) => value.clone(),
        Value::Null => String::new(),
        _ => serde_json::to_string(value).unwrap_or_else(|_| "<invalid value>".to_string()),
    })
}

fn one_line(value: impl AsRef<str>) -> String {
    value
        .as_ref()
        .replace(['\n', '\r', '\t'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn inline_params_must_be_json_object() {
        assert_eq!(
            load_params(Some(r#"{"days":30}"#), None).unwrap()["days"],
            30
        );
        assert!(load_params(Some("[1,2]"), None).is_err());
        assert!(load_params(Some("days: 30"), None).is_err());
    }

    #[test]
    fn params_file_accepts_json_and_yaml_objects() {
        let dir = TempDir::new().unwrap();
        let json = dir.path().join("params.json");
        let yaml = dir.path().join("params.yaml");
        fs::write(&json, r#"{"days":30}"#).unwrap();
        fs::write(&yaml, "days: 14\nactive: true\n").unwrap();
        assert_eq!(load_params(None, Some(&json)).unwrap()["days"], 30);
        assert_eq!(load_params(None, Some(&yaml)).unwrap()["days"], 14);
        assert_eq!(load_params(None, Some(&yaml)).unwrap()["active"], true);
    }

    #[test]
    fn text_cells_are_single_line_and_keep_types() {
        assert_eq!(
            display_value(&Value::String("one\ntwo".to_string())),
            "one two"
        );
        assert_eq!(display_value(&serde_json::json!(42)), "42");
        assert_eq!(display_value(&serde_json::json!(true)), "true");
        assert_eq!(display_value(&Value::Null), "");
    }
}

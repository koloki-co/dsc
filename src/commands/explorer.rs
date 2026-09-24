// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::api::{
    DiscourseClient, ExplorerParamInfo, ExplorerQueryDetails, ExplorerQuerySummary,
    ExplorerRunResult,
};
use crate::cli::ListFormat;
use crate::commands::common::{
    emit_result, ensure_api_credentials, fleet_worker_count, run_fleet, select_discourse,
    selected_discourses,
};
use crate::config::{Config, DiscourseConfig};
use crate::utils::{atomic_write_private, create_atomic_output};
use anyhow::{Context, Result, anyhow};
use serde::Serialize;
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

/// Forum and saved-query selection for Data Explorer execution.
pub struct ExplorerRunTarget<'a> {
    pub discourse: Option<&'a str>,
    pub all: bool,
    pub tags: Option<&'a str>,
    pub query_id: Option<i64>,
    pub query_name: Option<&'a str>,
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
    target: ExplorerRunTarget<'_>,
    options: ExplorerRunOptions<'_>,
    dry_run: bool,
) -> Result<()> {
    let params = load_params(options.params, options.params_file)?;
    if options.limit == Some(0) {
        return Err(anyhow!("--limit must be greater than zero"));
    }
    if query_name_is_empty(target.query_name) {
        return Err(anyhow!("--query-name must not be empty"));
    }

    let fleet = target.all || target.tags.is_some();
    if fleet {
        let query_name = target.query_name.ok_or_else(|| {
            anyhow!(
                "fleet Data Explorer execution requires --query-name; query IDs are forum-local"
            )
        })?;
        if options.csv.is_some() {
            return Err(anyhow!(
                "--csv is not supported with --all or --tags; use structured output until fleet destination semantics are defined"
            ));
        }
        return explorer_run_fleet(config, target.tags, query_name, &params, &options, dry_run);
    }

    let discourse_name = target
        .discourse
        .ok_or_else(|| anyhow!("a discourse name is required"))?;
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
        let query_label = query_label(target.query_id, target.query_name)?;
        println!(
            "[dry-run] {}: would run Data Explorer query {} ({} parameter{}) writing to {}",
            discourse.name,
            query_label,
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

    let query_id = resolve_query_id(&client, target.query_id, target.query_name)?;

    if let Some(path) = options.csv {
        let mut output = create_atomic_output(path, false, true)?;
        let bytes = client.download_explorer_query_csv(
            query_id,
            &params,
            options.limit,
            output.file_mut(),
        )?;
        output.commit()?;
        eprintln!(
            "Wrote Data Explorer CSV result to {} ({} bytes)",
            path.display(),
            bytes
        );
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

fn explorer_run_fleet(
    config: &Config,
    tags: Option<&str>,
    query_name: &str,
    params: &Map<String, Value>,
    options: &ExplorerRunOptions<'_>,
    dry_run: bool,
) -> Result<()> {
    let discourses = selected_discourses(config, None, tags)?;
    if discourses.is_empty() {
        return Err(if tags.is_some() {
            anyhow!("no discourses configured matching the given tags")
        } else {
            anyhow!("no discourses configured")
        });
    }

    if dry_run {
        for discourse in discourses {
            println!(
                "[dry-run] {}: would resolve and run exact Data Explorer query name {:?} ({} parameter{}) writing to stdout",
                discourse.name,
                query_name,
                params.len(),
                if params.len() == 1 { "" } else { "s" },
            );
        }
        if !params.is_empty() {
            println!("  params: {}", serde_json::to_string(params)?);
        }
        if let Some(limit) = options.limit {
            println!("  limit: {limit}");
        }
        if options.explain {
            println!("  explain: requested");
        }
        return Ok(());
    }

    let query_name = query_name.to_string();
    let params = params.clone();
    let explain = options.explain;
    let limit = options.limit;
    let results: Vec<FleetExplorerRunResult> = run_fleet(
        &discourses,
        fleet_worker_count(None, discourses.len(), 8, false),
        |discourse| run_named_query_one(discourse, &query_name, &params, explain, limit),
        |result| {
            if let FleetExplorerRunResult::Failure { forum, error, .. } = result {
                eprintln!("{forum}: Data Explorer query failed - {error}");
            }
        },
    );

    let failed = results
        .iter()
        .filter(|result| matches!(result, FleetExplorerRunResult::Failure { .. }))
        .count();
    let text = render_fleet_results(&results);
    emit_result(options.format, &results, &text)?;

    if failed > 0 {
        return Err(anyhow!(
            "Data Explorer query failed on {failed} of {} forum(s)",
            discourses.len()
        ));
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(untagged)]
enum FleetExplorerRunResult {
    Success {
        forum: String,
        query_id: i64,
        query_name: String,
        result: ExplorerRunResult,
    },
    Failure {
        forum: String,
        query_name: String,
        error: String,
    },
}

fn run_named_query_one(
    discourse: &DiscourseConfig,
    query_name: &str,
    params: &Map<String, Value>,
    explain: bool,
    limit: Option<u32>,
) -> FleetExplorerRunResult {
    let result = (|| {
        ensure_api_credentials(discourse)?;
        let client = DiscourseClient::new(discourse)?;
        let query_id = resolve_exact_query_name(&client, query_name)?;
        let result = client.run_explorer_query(query_id, params, explain, limit)?;
        Ok::<_, anyhow::Error>((query_id, result))
    })();

    match result {
        Ok((query_id, result)) => FleetExplorerRunResult::Success {
            forum: discourse.name.clone(),
            query_id,
            query_name: query_name.to_string(),
            result,
        },
        Err(error) => FleetExplorerRunResult::Failure {
            forum: discourse.name.clone(),
            query_name: query_name.to_string(),
            error: error.to_string(),
        },
    }
}

fn query_name_is_empty(query_name: Option<&str>) -> bool {
    query_name.is_some_and(str::is_empty)
}

fn resolve_query_id(
    client: &DiscourseClient,
    query_id: Option<i64>,
    query_name: Option<&str>,
) -> Result<i64> {
    match (query_id, query_name) {
        (Some(query_id), None) => Ok(query_id),
        (None, Some(query_name)) => resolve_exact_query_name(client, query_name),
        _ => Err(anyhow!("use exactly one of a query ID or --query-name")),
    }
}

fn resolve_exact_query_name(client: &DiscourseClient, query_name: &str) -> Result<i64> {
    let catalogue = client.list_explorer_queries(Some(query_name), Some("name"), true)?;
    let matches: Vec<_> = catalogue
        .queries
        .iter()
        .filter(|query| query.name == query_name)
        .collect();
    match matches.as_slice() {
        [query] => Ok(query.id),
        [] => Err(anyhow!(
            "Data Explorer query not found by exact name: {query_name}"
        )),
        matches => Err(anyhow!(
            "multiple Data Explorer queries have the exact name {:?}: IDs {}",
            query_name,
            matches
                .iter()
                .map(|query| query.id.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn query_label(query_id: Option<i64>, query_name: Option<&str>) -> Result<String> {
    match (query_id, query_name) {
        (Some(query_id), None) => Ok(query_id.to_string()),
        (None, Some(query_name)) => Ok(format!("named {:?}", query_name)),
        _ => Err(anyhow!("use exactly one of a query ID or --query-name")),
    }
}

fn render_fleet_results(results: &[FleetExplorerRunResult]) -> String {
    let mut output = String::new();
    for (index, result) in results.iter().enumerate() {
        if index > 0 {
            output.push('\n');
        }
        match result {
            FleetExplorerRunResult::Success {
                forum,
                query_id,
                query_name,
                result,
            } => {
                output.push_str(&format!(
                    "== {forum} ==\nquery: {query_name} ({query_id})\n"
                ));
                output.push_str(&render_run_result(result));
            }
            FleetExplorerRunResult::Failure { forum, error, .. } => {
                output.push_str(&format!("== {forum} ==\nerror: {error}\n"));
            }
        }
    }
    output
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
    print!("{}", render_run_result(result));
    let duration = result
        .duration
        .map(|milliseconds| format!(", {milliseconds:.1} ms"))
        .unwrap_or_default();
    eprintln!("{} row(s){duration}", result.rows.len());
}

fn render_run_result(result: &ExplorerRunResult) -> String {
    let mut output = String::new();
    let column_count = result
        .rows
        .iter()
        .map(Vec::len)
        .max()
        .unwrap_or(0)
        .max(result.columns.len());
    if column_count == 0 {
        output.push_str("No rows returned.\n");
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
        render_table_row(&mut output, &headers, &widths);
        output.push_str(&format!(
            "{}\n",
            widths
                .iter()
                .map(|width| "-".repeat(*width))
                .collect::<Vec<_>>()
                .join("  ")
        ));
        for row in &rendered_rows {
            render_table_row(&mut output, row, &widths);
        }
    }
    if let Some(explain) = &result.explain {
        output.push_str("\nexplain:\n");
        output.push_str(explain);
        output.push('\n');
    }
    output
}

fn render_table_row(output: &mut String, cells: &[String], widths: &[usize]) {
    output.push_str(&format!(
        "{}\n",
        cells
            .iter()
            .enumerate()
            .map(|(index, cell)| format!("{cell:<width$}", width = widths[index]))
            .collect::<Vec<_>>()
            .join("  ")
    ));
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

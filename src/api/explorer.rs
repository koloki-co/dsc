// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use super::client::{DiscourseClient, ResponseBody};
use super::error::http_error;
use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashSet};

const QUERIES_PATH: &str = "/admin/plugins/discourse-data-explorer/queries.json";
const MAX_PAGES: usize = 1_000;

/// One saved Data Explorer query in list output.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExplorerQuerySummary {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub group_ids: Vec<i64>,
    #[serde(default)]
    pub last_run_at: Option<String>,
    #[serde(default)]
    pub user_id: Option<i64>,
    #[serde(default)]
    pub is_default: bool,
}

/// Fully paginated Data Explorer query catalogue.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExplorerQueryCatalogue {
    pub queries: Vec<ExplorerQuerySummary>,
    pub total_rows_queries: usize,
}

#[derive(Debug, Deserialize)]
struct ExplorerQueryPage {
    queries: Vec<ExplorerQuerySummary>,
    total_rows_queries: usize,
    #[serde(default)]
    load_more_queries: Option<String>,
}

/// One declared Data Explorer query parameter.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExplorerParamInfo {
    pub identifier: String,
    #[serde(rename = "type")]
    pub param_type: String,
    #[serde(default)]
    pub default: Option<Value>,
    #[serde(default)]
    pub nullable: bool,
    #[serde(default)]
    pub internal: bool,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Saved Data Explorer query definition returned by the admin endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExplorerQueryDetails {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub group_ids: Vec<i64>,
    #[serde(default)]
    pub last_run_at: Option<String>,
    #[serde(default)]
    pub user_id: Option<i64>,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default)]
    pub sql: Option<String>,
    #[serde(default)]
    pub param_info: Vec<ExplorerParamInfo>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub cached_result: Option<ExplorerRunResult>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Deserialize)]
struct ExplorerQueryEnvelope {
    query: ExplorerQueryDetails,
}

/// Result of executing one saved Data Explorer query.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExplorerRunResult {
    #[serde(default)]
    pub success: bool,
    #[serde(default)]
    pub errors: Vec<String>,
    #[serde(default)]
    pub params: Map<String, Value>,
    #[serde(default)]
    pub duration: Option<f64>,
    #[serde(default)]
    pub columns: Vec<String>,
    #[serde(default)]
    pub rows: Vec<Vec<Value>>,
    #[serde(default)]
    pub explain: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl DiscourseClient {
    /// List every accessible saved Data Explorer query, following pagination.
    pub fn list_explorer_queries(
        &self,
        filter: Option<&str>,
        order: Option<&str>,
        ascending: bool,
    ) -> Result<ExplorerQueryCatalogue> {
        let mut next = Some(build_list_path(filter, order, ascending)?);
        let mut seen_paths = HashSet::new();
        let mut seen_ids = HashSet::new();
        let mut queries = Vec::new();
        let mut total_rows_queries = 0;
        let mut pages = 0;

        while let Some(path) = next {
            validate_page_path(&path)?;
            if !seen_paths.insert(path.clone()) {
                return Err(anyhow!("Data Explorer pagination loop at {path}"));
            }
            pages += 1;
            if pages > MAX_PAGES {
                return Err(anyhow!(
                    "Data Explorer pagination exceeded {MAX_PAGES} pages"
                ));
            }

            let response = self.get(&path)?;
            let status = response.status();
            let text = response
                .text_capped()
                .context("reading Data Explorer query list response")?;
            if !status.is_success() {
                return Err(explorer_http_error(
                    "Data Explorer query list",
                    status,
                    &text,
                ));
            }
            let page: ExplorerQueryPage =
                serde_json::from_str(&text).context("parsing Data Explorer query list response")?;
            total_rows_queries = page.total_rows_queries;
            for query in page.queries {
                if seen_ids.insert(query.id) {
                    queries.push(query);
                }
            }
            next = normalize_next_page(page.load_more_queries);
        }

        Ok(ExplorerQueryCatalogue {
            queries,
            total_rows_queries,
        })
    }

    /// Fetch one saved Data Explorer query definition.
    pub fn show_explorer_query(&self, query_id: i64) -> Result<ExplorerQueryDetails> {
        let path = format!("/admin/plugins/discourse-data-explorer/queries/{query_id}.json");
        let response = self.get(&path)?;
        let status = response.status();
        let text = response
            .text_capped()
            .context("reading Data Explorer query response")?;
        if !status.is_success() {
            return Err(explorer_http_error("Data Explorer query", status, &text));
        }
        let envelope: ExplorerQueryEnvelope =
            serde_json::from_str(&text).context("parsing Data Explorer query response")?;
        Ok(envelope.query)
    }

    /// Download the server's exact portable query-definition export.
    pub fn export_explorer_query(&self, query_id: i64) -> Result<Vec<u8>> {
        let path =
            format!("/admin/plugins/discourse-data-explorer/queries/{query_id}.json?export=true");
        response_bytes(self.get(&path)?, "Data Explorer query export")
    }

    /// Execute one saved Data Explorer query and return its typed JSON result.
    pub fn run_explorer_query(
        &self,
        query_id: i64,
        params: &Map<String, Value>,
        explain: bool,
        limit: Option<u32>,
    ) -> Result<ExplorerRunResult> {
        let path = format!("/admin/plugins/discourse-data-explorer/queries/{query_id}/run.json");
        let payload = run_payload(params, explain, limit)?;
        let response = self.send_retrying(|| Ok(self.post(&path)?.form(&payload)))?;
        let status = response.status();
        let text = response
            .text_capped()
            .context("reading Data Explorer query result")?;
        if !status.is_success() {
            return Err(explorer_http_error(
                "Data Explorer query run",
                status,
                &text,
            ));
        }
        let result: ExplorerRunResult =
            serde_json::from_str(&text).context("parsing Data Explorer query result")?;
        if !result.success {
            return Err(anyhow!(
                "Data Explorer query failed: {}",
                result.errors.join("; ")
            ));
        }
        Ok(result)
    }

    /// Execute one saved query and stream the server-generated CSV directly
    /// into `writer`, returning the number of bytes written. Avoids holding
    /// the complete CSV (which can contain large text cells) in memory
    /// before it reaches disk.
    pub fn download_explorer_query_csv(
        &self,
        query_id: i64,
        params: &Map<String, Value>,
        limit: Option<u32>,
        writer: &mut impl std::io::Write,
    ) -> Result<u64> {
        let path = format!("/admin/plugins/discourse-data-explorer/queries/{query_id}/run.csv");
        let mut payload = run_payload(params, false, limit)?;
        payload.push(("download", "1".to_string()));
        let mut response = self.send_retrying(|| Ok(self.post(&path)?.form(&payload)))?;
        let status = response.status();
        if !status.is_success() {
            let text = response
                .text_capped()
                .context("reading Data Explorer CSV download response")?;
            return Err(explorer_http_error(
                "Data Explorer CSV download",
                status,
                &text,
            ));
        }
        response
            .copy_to(writer)
            .context("streaming Data Explorer CSV download")
    }
}

fn build_list_path(filter: Option<&str>, order: Option<&str>, ascending: bool) -> Result<String> {
    let mut url = reqwest::Url::parse(&format!("https://dsc.invalid{QUERIES_PATH}"))?;
    {
        let mut pairs = url.query_pairs_mut();
        if let Some(filter) = filter.filter(|value| !value.is_empty()) {
            pairs.append_pair("filter", filter);
        }
        if let Some(order) = order {
            pairs.append_pair("order", order);
        }
        if ascending {
            pairs.append_pair("ascending", "true");
        }
    }
    Ok(match url.query() {
        Some(query) if !query.is_empty() => format!("{}?{query}", url.path()),
        None => url.path().to_string(),
        Some(_) => url.path().to_string(),
    })
}

fn validate_page_path(path: &str) -> Result<()> {
    if path == QUERIES_PATH
        || path
            .strip_prefix(&format!("{QUERIES_PATH}?"))
            .is_some_and(|query| !query.is_empty() && !query.contains('#'))
    {
        return Ok(());
    }
    Err(anyhow!(
        "refusing unexpected Data Explorer pagination path: {path}"
    ))
}

fn normalize_next_page(path: Option<String>) -> Option<String> {
    path.filter(|path| !path.is_empty() && path != &format!("{QUERIES_PATH}?"))
}

fn run_payload(
    params: &Map<String, Value>,
    explain: bool,
    limit: Option<u32>,
) -> Result<Vec<(&'static str, String)>> {
    let mut payload = vec![("params", serde_json::to_string(params)?)];
    if explain {
        payload.push(("explain", "true".to_string()));
    }
    if let Some(limit) = limit {
        payload.push(("limit", limit.to_string()));
    }
    Ok(payload)
}

fn response_bytes(response: reqwest::blocking::Response, action: &str) -> Result<Vec<u8>> {
    let status = response.status();
    let bytes = response
        .bytes()
        .with_context(|| format!("reading {action} response"))?;
    if !status.is_success() {
        let text = String::from_utf8_lossy(&bytes);
        return Err(explorer_http_error(action, status, &text));
    }
    Ok(bytes.to_vec())
}

fn explorer_http_error(action: &str, status: reqwest::StatusCode, text: &str) -> anyhow::Error {
    if status == reqwest::StatusCode::NOT_FOUND {
        return anyhow!(
            "{}; Data Explorer may be disabled, unavailable on this Discourse version, or the query may be hidden/inaccessible",
            http_error(action, status, text)
        );
    }
    http_error(action, status, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_path_encodes_filter_and_sort() {
        assert_eq!(build_list_path(None, None, false).unwrap(), QUERIES_PATH);
        assert_eq!(
            build_list_path(Some("mail & notifications"), Some("name"), true).unwrap(),
            concat!(
                "/admin/plugins/discourse-data-explorer/queries.json?",
                "filter=mail+%26+notifications&order=name&ascending=true"
            )
        );
    }

    #[test]
    fn pagination_path_is_restricted_to_canonical_query_list() {
        assert!(validate_page_path(QUERIES_PATH).is_ok());
        assert!(validate_page_path(&format!("{QUERIES_PATH}?offset=50")).is_ok());
        assert!(validate_page_path("https://evil.example/queries.json?offset=50").is_err());
        assert!(validate_page_path("/admin/plugins/explorer/queries.json?offset=50").is_err());
    }

    #[test]
    fn empty_query_string_is_a_legacy_end_of_pagination_sentinel() {
        assert_eq!(normalize_next_page(Some(format!("{QUERIES_PATH}?"))), None);
        assert_eq!(
            normalize_next_page(Some(format!("{QUERIES_PATH}?offset=50"))),
            Some(format!("{QUERIES_PATH}?offset=50"))
        );
    }

    #[test]
    fn current_result_shape_preserves_typed_cells_and_extra_metadata() {
        let result: ExplorerRunResult = serde_json::from_str(
            r#"{
                "success": true,
                "errors": [],
                "params": {"days": 30},
                "duration": 12.4,
                "columns": ["name", "count", "active"],
                "rows": [["alice", 2, true]],
                "relations": {"user": []}
            }"#,
        )
        .unwrap();
        assert_eq!(result.rows[0][1], serde_json::json!(2));
        assert_eq!(result.rows[0][2], serde_json::json!(true));
        assert!(result.extra.contains_key("relations"));
    }

    #[test]
    fn negative_default_query_id_deserializes() {
        let query: ExplorerQuerySummary = serde_json::from_str(
            r#"{"id":-1,"name":"Top topics","group_ids":[],"is_default":true}"#,
        )
        .unwrap();
        assert_eq!(query.id, -1);
        assert!(query.is_default);
    }
}

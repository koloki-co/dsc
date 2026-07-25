// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use super::client::DiscourseClient;
use super::error::http_error;
use super::models::{
    CategoriesResponse, CategoryDefinition, CategoryDefinitionsResponse, CategoryInfo,
    CategoryResponse, CreateCategoryResponse,
};
use anyhow::{Context, Result, anyhow};
use reqwest::StatusCode;
use serde_json::Value;
use std::collections::HashMap;

impl DiscourseClient {
    /// Fetch a category by ID (topics list included).
    pub fn fetch_category(&self, category_id: u64) -> Result<CategoryResponse> {
        let path = format!("/c/{}.json", category_id);
        let response = self.get(&path)?;
        let status = response.status();
        let text = response.text().context("reading category response body")?;
        if !status.is_success() {
            if status == StatusCode::NOT_FOUND {
                return Err(anyhow!("category not found: {}", category_id));
            }
            return Err(http_error("category request", status, &text));
        }
        let body: CategoryResponse =
            serde_json::from_str(&text).context("reading category json")?;
        Ok(body)
    }

    /// Fetch all categories.
    pub fn fetch_categories(&self) -> Result<Vec<CategoryInfo>> {
        let response = self.get("/categories.json?include_subcategories=true")?;
        let status = response.status();
        let text = response
            .text()
            .context("reading categories response body")?;
        if !status.is_success() {
            return Err(http_error("categories request", status, &text));
        }
        let body: CategoriesResponse =
            serde_json::from_str(&text).context("reading categories json")?;
        let mut categories = body.category_list.categories;
        if let Ok(site_categories) = self.fetch_site_categories() {
            let mut seen = HashMap::new();
            for (idx, cat) in categories.iter().enumerate() {
                if let Some(id) = cat.id {
                    seen.insert(id, idx);
                }
            }
            for cat in site_categories {
                if let Some(id) = cat.id
                    && !seen.contains_key(&id)
                {
                    categories.push(cat);
                }
            }
        }
        Ok(categories)
    }

    /// Create a category with basic fields copied from a source category.
    pub fn create_category(&self, category: &CategoryInfo) -> Result<u64> {
        let mut payload = vec![("name", category.name.clone())];
        if !category.slug.is_empty() {
            payload.push(("slug", category.slug.clone()));
        }
        if let Some(color) = category.color.clone() {
            payload.push(("color", color));
        }
        if let Some(text_color) = category.text_color.clone() {
            payload.push(("text_color", text_color));
        }
        let response = self.send_retrying(|| Ok(self.post("/categories")?.form(&payload)))?;
        let status = response.status();
        let text = response.text().context("reading category response body")?;
        if !status.is_success() {
            return Err(http_error("create category request", status, &text));
        }
        let body: CreateCategoryResponse =
            serde_json::from_str(&text).context("reading category response")?;
        Ok(body.category.id)
    }

    /// Fetch the full definition of every category, including group permissions.
    /// This is the read side of `category def pull` / `show` / `get`; unlike
    /// `fetch_categories` it carries description, permissions, topic template,
    /// tag rules, and ordering.
    pub fn fetch_category_definitions(&self) -> Result<Vec<CategoryDefinition>> {
        let response =
            self.get("/categories.json?show_permissions=true&include_subcategories=true")?;
        let status = response.status();
        let text = response
            .text()
            .context("reading category definitions response body")?;
        if !status.is_success() {
            return Err(http_error("category definitions request", status, &text));
        }
        let body: CategoryDefinitionsResponse =
            serde_json::from_str(&text).context("reading category definitions json")?;
        Ok(body.category_list.categories)
    }

    /// Create a category from raw form params, returning the new category's ID.
    /// The caller assembles the definition params (name is required); this is
    /// the full-definition counterpart to [`create_category`], which sends only
    /// the four fields `category copy` needs.
    pub fn create_category_def(&self, params: &[(String, String)]) -> Result<u64> {
        let response = self.send_retrying(|| Ok(self.post("/categories")?.form(params)))?;
        let status = response.status();
        let text = response
            .text()
            .context("reading create category response body")?;
        if !status.is_success() {
            return Err(http_error("create category request", status, &text));
        }
        let body: CreateCategoryResponse =
            serde_json::from_str(&text).context("reading create category response")?;
        Ok(body.category.id)
    }

    /// Update a category's definition from raw form params
    /// (`PUT /categories/{id}.json`). The endpoint missing from `dsc` until now.
    pub fn update_category(&self, id: u64, params: &[(String, String)]) -> Result<()> {
        let path = format!("/categories/{}.json", id);
        let response = self.send_retrying(|| Ok(self.put(&path)?.form(params)))?;
        let status = response.status();
        let text = response
            .text()
            .context("reading update category response body")?;
        if !status.is_success() {
            return Err(http_error("update category request", status, &text));
        }
        Ok(())
    }

    fn fetch_site_categories(&self) -> Result<Vec<CategoryInfo>> {
        let response = self.get("/site.json")?;
        let status = response.status();
        let text = response.text().context("reading site.json response body")?;
        if !status.is_success() {
            return Err(http_error("site.json request", status, &text));
        }
        let value: Value = serde_json::from_str(&text).context("parsing site.json")?;
        let array = value
            .get("categories")
            .and_then(|v| v.as_array())
            .or_else(|| {
                value
                    .get("site")
                    .and_then(|v| v.get("categories"))
                    .and_then(|v| v.as_array())
            })
            .ok_or_else(|| anyhow!("site.json missing categories list"))?;
        let mut categories = Vec::new();
        for item in array {
            if let Ok(cat) = serde_json::from_value::<CategoryInfo>(item.clone()) {
                categories.push(cat);
            }
        }
        Ok(categories)
    }
}

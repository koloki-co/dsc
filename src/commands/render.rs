// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::cli::ListFormat;
use crate::commands::common::{emit_result, select_discourse};
use crate::config::{Config, DiscourseConfig};
use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read};
use std::path::Path;

#[derive(Serialize)]
struct RenderOutput {
    rendered: String,
}

/// Render `{{ variable }}` placeholders in `file` using `discourse_name`'s
/// resolved template variables (built-ins, `[template.vars]` globals, and
/// the forum's own `[discourse.template]` overrides).
pub fn render(
    config: &Config,
    discourse_name: &str,
    file: &Path,
    output: Option<&Path>,
    format: ListFormat,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    let content = read_template_input(file)?;
    let vars = resolve_template_vars(config, discourse);
    let rendered = render_template(&content, &vars)?;

    if dry_run {
        eprintln!("Resolved template variables:");
        for (key, value) in &vars {
            eprintln!("  {} = {}", key, value);
        }
        return emit_result(
            format,
            &RenderOutput {
                rendered: rendered.clone(),
            },
            &rendered,
        );
    }

    match output {
        Some(path) => {
            fs::write(path, &rendered).with_context(|| format!("writing {}", path.display()))?;
            println!("Wrote rendered output to {}", path.display());
            Ok(())
        }
        None => emit_result(
            format,
            &RenderOutput {
                rendered: rendered.clone(),
            },
            &rendered,
        ),
    }
}

fn read_template_input(file: &Path) -> Result<String> {
    if file.as_os_str() == "-" {
        let mut buf = String::new();
        io::stdin()
            .read_to_string(&mut buf)
            .context("reading template from stdin")?;
        Ok(buf)
    } else {
        fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))
    }
}

/// Resolve the full template variable map for one forum: built-ins first,
/// then `[template.vars]` globals, then the forum's own `[discourse.template]`
/// (each layer overriding the previous on a same-name key).
fn resolve_template_vars(config: &Config, discourse: &DiscourseConfig) -> BTreeMap<String, String> {
    let mut vars = BTreeMap::new();
    vars.insert("forum_baseurl".to_string(), discourse.baseurl.clone());
    vars.insert("forum_name".to_string(), discourse.name.clone());
    vars.insert(
        "forum_fullname".to_string(),
        discourse.fullname.as_deref().unwrap_or("").to_string(),
    );
    for (key, value) in &config.template.vars {
        vars.insert(key.clone(), value.clone());
    }
    for (key, value) in &discourse.template {
        vars.insert(key.clone(), value.clone());
    }
    vars
}

/// Render `{{ variable }}` interpolations via Tera. Unlike Tera's default
/// (which errors on an undefined variable), an unknown variable here warns
/// to stderr and substitutes an empty string, per the render spec.
fn render_template(content: &str, vars: &BTreeMap<String, String>) -> Result<String> {
    let mut context = tera::Context::new();
    for (key, value) in vars {
        context.insert(key.clone(), value);
    }
    for name in referenced_variable_names(content) {
        if !vars.contains_key(&name) {
            eprintln!(
                "warning: unknown template variable '{}', substituting empty string",
                name
            );
            context.insert(name, "");
        }
    }
    tera::Tera::one_off(content, &context, false).context("rendering template")
}

/// Extract the plain identifiers referenced by `{{ identifier }}` in
/// `content`. Only bare single-identifier interpolations are recognised
/// (Phase 1 scope); anything with filters, dots, or expressions is left for
/// Tera itself to resolve or reject at render time.
fn referenced_variable_names(content: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = content;
    while let Some(start) = rest.find("{{") {
        let after_open = &rest[start + 2..];
        let Some(end) = after_open.find("}}") else {
            break;
        };
        let inner = after_open[..end].trim();
        if is_plain_identifier(inner) {
            names.push(inner.to_string());
        }
        rest = &after_open[end + 2..];
    }
    names
}

fn is_plain_identifier(candidate: &str) -> bool {
    let mut chars = candidate.chars();
    match chars.next() {
        Some(ch) if ch.is_ascii_alphabetic() || ch == '_' => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_plain_identifiers() {
        let content = "Hi {{ community }}, visit {{forum_baseurl}}/about.";
        assert_eq!(
            referenced_variable_names(content),
            vec!["community".to_string(), "forum_baseurl".to_string()]
        );
    }

    #[test]
    fn ignores_non_plain_expressions_and_percent_placeholders() {
        let content = "%{reply_to_username,fallback:there} {{ community | default(\"x\") }}";
        assert!(referenced_variable_names(content).is_empty());
    }

    #[test]
    fn renders_known_variables() {
        let mut vars = BTreeMap::new();
        vars.insert("community".to_string(), "OpenEHR".to_string());
        let out = render_template("Welcome to {{ community }}!", &vars).unwrap();
        assert_eq!(out, "Welcome to OpenEHR!");
    }

    #[test]
    fn substitutes_empty_string_for_unknown_variables() {
        let vars = BTreeMap::new();
        let out = render_template("Hello {{ missing }}!", &vars).unwrap();
        assert_eq!(out, "Hello !");
    }

    #[test]
    fn built_in_vars_derive_from_discourse_config() {
        let mut config = Config::default();
        config
            .template
            .vars
            .insert("organisation".to_string(), "Koloki Ltd".to_string());
        let discourse = DiscourseConfig {
            name: "openehr".to_string(),
            baseurl: "https://discourse.openehr.org".to_string(),
            fullname: Some("openEHR International".to_string()),
            ..DiscourseConfig::default()
        };
        let vars = resolve_template_vars(&config, &discourse);
        assert_eq!(
            vars.get("forum_baseurl").unwrap(),
            "https://discourse.openehr.org"
        );
        assert_eq!(vars.get("forum_name").unwrap(), "openehr");
        assert_eq!(vars.get("forum_fullname").unwrap(), "openEHR International");
        assert_eq!(vars.get("organisation").unwrap(), "Koloki Ltd");
    }

    #[test]
    fn per_forum_template_overrides_global_vars() {
        let mut config = Config::default();
        config
            .template
            .vars
            .insert("organisation".to_string(), "Koloki Ltd".to_string());
        let mut discourse = DiscourseConfig {
            name: "openehr".to_string(),
            baseurl: "https://discourse.openehr.org".to_string(),
            ..DiscourseConfig::default()
        };
        discourse.template.insert(
            "organisation".to_string(),
            "openEHR International".to_string(),
        );
        let vars = resolve_template_vars(&config, &discourse);
        assert_eq!(vars.get("organisation").unwrap(), "openEHR International");
    }
}

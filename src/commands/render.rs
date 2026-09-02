// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::cli::ListFormat;
use crate::commands::common::{emit_result, select_discourse};
use crate::config::{Config, DiscourseConfig};
use crate::utils::atomic_write;
use anyhow::{Context, Result, anyhow};
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
#[allow(clippy::too_many_arguments)]
pub fn render(
    config: &Config,
    discourse_name: &str,
    file: Option<&Path>,
    output: Option<&Path>,
    strict: bool,
    list_vars: bool,
    format: ListFormat,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    let vars = resolve_template_vars(config, discourse);

    if list_vars {
        let text = vars
            .iter()
            .map(|(key, value)| format!("{} = {}", key, value))
            .collect::<Vec<_>>()
            .join("\n");
        return emit_result(format, &vars, &text);
    }

    // Clap enforces a file argument whenever `--list-vars` is absent.
    let file = file.ok_or_else(|| anyhow!("missing template file"))?;
    let content = read_template_input(file)?;
    let rendered = render_template(&content, &vars, strict)?;

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
            atomic_write(path, &rendered, true)?;
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

/// Render `content` against `discourse`'s resolved template variables, for
/// callers integrating rendering into another command (the `--render` flag
/// on `topic new`/`topic push`/`topic reply`/`category push`) rather than
/// the standalone `dsc render` command. Non-strict: an unknown variable
/// warns to stderr and substitutes an empty string, matching `dsc render`'s
/// default behaviour.
pub fn render_content(
    config: &Config,
    discourse: &DiscourseConfig,
    content: &str,
) -> Result<String> {
    let vars = resolve_template_vars(config, discourse);
    render_template(content, &vars, false)
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
/// to stderr and substitutes an empty string, per the render spec. Under
/// `strict`, every unknown variable is collected and reported as one error
/// instead, so a caller fixes the whole template in a single pass.
///
/// Content inside Markdown code fences (``` ``` ``` or `~~~`) is passed
/// through byte-for-byte: it is neither validated nor substituted, so a
/// template's own documentation of `{{ }}`/`{% %}` syntax in a fenced
/// example does not trip Phase 1 validation or get rewritten.
fn render_template(content: &str, vars: &BTreeMap<String, String>, strict: bool) -> Result<String> {
    let segments = split_code_fences(content);

    let mut unknown: Vec<String> = Vec::new();
    for segment in &segments {
        if let Segment::Text(text) = segment {
            validate_phase_one_syntax(text)?;
            for name in referenced_variable_names(text) {
                if vars.contains_key(&name) || unknown.contains(&name) {
                    continue;
                }
                unknown.push(name);
            }
        }
    }

    if strict && !unknown.is_empty() {
        return Err(anyhow!(
            "unknown template variable(s): {}",
            unknown.join(", ")
        ));
    }

    let mut context = tera::Context::new();
    for (key, value) in vars {
        context.insert(key.clone(), value);
    }
    for name in &unknown {
        eprintln!(
            "warning: unknown template variable '{}', substituting empty string",
            name
        );
        context.insert(name.clone(), "");
    }

    let mut rendered = String::with_capacity(content.len());
    for segment in segments {
        match segment {
            Segment::Text(text) => {
                let piece =
                    tera::Tera::one_off(&text, &context, false).context("rendering template")?;
                rendered.push_str(&piece);
            }
            Segment::Fence(raw) => rendered.push_str(&raw),
        }
    }
    Ok(rendered)
}

/// A chunk of template content: either plain `Text` (subject to Phase 1
/// validation and `{{ variable }}` substitution) or a raw Markdown code
/// `Fence` (an opening delimiter line through its matching closing line,
/// or through end-of-file if unterminated), copied through untouched.
enum Segment {
    Text(String),
    Fence(String),
}

/// Split `content` into alternating `Text`/`Fence` segments on Markdown
/// fenced code blocks: a line whose stripped content starts with three or
/// more backticks or tildes opens a fence, closed by a line starting with
/// at least as many of the same character and nothing else but whitespace.
/// An unterminated fence runs to end-of-file, matching CommonMark.
fn split_code_fences(content: &str) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut text = String::new();
    let mut fence: Option<(char, usize, String)> = None;

    for line in content.split_inclusive('\n') {
        let stripped = line.trim_end_matches(['\n', '\r']).trim_start();
        match &mut fence {
            None => {
                if let Some((ch, len)) = fence_open(stripped) {
                    if !text.is_empty() {
                        segments.push(Segment::Text(std::mem::take(&mut text)));
                    }
                    fence = Some((ch, len, line.to_string()));
                } else {
                    text.push_str(line);
                }
            }
            Some((ch, len, buf)) => {
                buf.push_str(line);
                if is_fence_close(stripped, *ch, *len) {
                    let (_, _, buf) = fence.take().unwrap();
                    segments.push(Segment::Fence(buf));
                }
            }
        }
    }

    match fence {
        Some((_, _, buf)) => segments.push(Segment::Fence(buf)),
        None if !text.is_empty() => segments.push(Segment::Text(text)),
        None => {}
    }
    segments
}

/// If `stripped` opens a code fence, return its delimiter character and run
/// length (3+ backticks or tildes required).
fn fence_open(stripped: &str) -> Option<(char, usize)> {
    let ch = stripped.chars().next()?;
    if ch != '`' && ch != '~' {
        return None;
    }
    let len = stripped.chars().take_while(|&c| c == ch).count();
    if len < 3 {
        return None;
    }
    Some((ch, len))
}

/// Whether `stripped` closes a fence opened with `fence_char`/`fence_len`:
/// at least `fence_len` repeats of `fence_char`, then only whitespace.
fn is_fence_close(stripped: &str, fence_char: char, fence_len: usize) -> bool {
    let len = stripped.chars().take_while(|&c| c == fence_char).count();
    len >= fence_len && stripped.chars().skip(len).all(|c| c.is_whitespace())
}

/// Reject Tera features outside Phase 1 before they become an accidental
/// supported surface. The engine remains in place for the planned expansion.
fn validate_phase_one_syntax(content: &str) -> Result<()> {
    if content.contains("{%") || content.contains("{#") {
        return Err(anyhow!(
            "Phase 1 templates support only bare {{ variable }} interpolation; Tera statements and comments are not supported"
        ));
    }

    let mut rest = content;
    while let Some(start) = rest.find("{{") {
        let after_open = &rest[start + 2..];
        let Some(end) = after_open.find("}}") else {
            return Err(anyhow!("unterminated template interpolation"));
        };
        let inner = after_open[..end].trim();
        if !is_plain_identifier(inner) {
            return Err(anyhow!(
                "Phase 1 templates support only bare {{ variable }} interpolation, got {{{{{}}}}}",
                inner
            ));
        }
        rest = &after_open[end + 2..];
    }
    Ok(())
}

/// Extract the plain identifiers referenced by `{{ identifier }}` in
/// `content`. `validate_phase_one_syntax` has already rejected expressions.
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
    fn ignores_percent_placeholders() {
        let content = "%{reply_to_username,fallback:there}";
        assert!(referenced_variable_names(content).is_empty());
    }

    #[test]
    fn rejects_filters_and_control_blocks() {
        let mut vars = BTreeMap::new();
        vars.insert("community".to_string(), "OpenEHR".to_string());
        assert!(render_template("{{ community | upper }}", &vars, false).is_err());
        assert!(render_template("{% if community %}Welcome{% endif %}", &vars, false).is_err());
    }

    #[test]
    fn renders_known_variables() {
        let mut vars = BTreeMap::new();
        vars.insert("community".to_string(), "OpenEHR".to_string());
        let out = render_template("Welcome to {{ community }}!", &vars, false).unwrap();
        assert_eq!(out, "Welcome to OpenEHR!");
    }

    #[test]
    fn substitutes_empty_string_for_unknown_variables() {
        let vars = BTreeMap::new();
        let out = render_template("Hello {{ missing }}!", &vars, false).unwrap();
        assert_eq!(out, "Hello !");
    }

    #[test]
    fn strict_reports_every_unknown_variable_once() {
        let vars = BTreeMap::new();
        let err = render_template("{{ a }} {{ b }} {{ a }}", &vars, true).unwrap_err();
        assert_eq!(err.to_string(), "unknown template variable(s): a, b");
    }

    #[test]
    fn strict_renders_a_fully_resolved_template() {
        let mut vars = BTreeMap::new();
        vars.insert("community".to_string(), "OpenEHR".to_string());
        let out = render_template("Welcome to {{ community }}!", &vars, true).unwrap();
        assert_eq!(out, "Welcome to OpenEHR!");
    }

    #[test]
    fn render_content_substitutes_discourse_template_vars() {
        let config = Config::default();
        let mut discourse = DiscourseConfig {
            name: "openehr".to_string(),
            baseurl: "https://discourse.openehr.org".to_string(),
            ..DiscourseConfig::default()
        };
        discourse.template.insert(
            "organisation".to_string(),
            "openEHR International".to_string(),
        );
        let out =
            render_content(&config, &discourse, "Brought to you by {{ organisation }}.").unwrap();
        assert_eq!(out, "Brought to you by openEHR International.");
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
    fn code_fence_content_is_not_substituted() {
        let mut vars = BTreeMap::new();
        vars.insert("community".to_string(), "OpenEHR".to_string());
        let content = "Welcome to {{ community }}!\n\n```\nExample: {{ community }}\n```\n";
        let out = render_template(content, &vars, false).unwrap();
        assert_eq!(
            out,
            "Welcome to OpenEHR!\n\n```\nExample: {{ community }}\n```\n"
        );
    }

    #[test]
    fn code_fence_unknown_variables_are_not_warned_or_counted() {
        let vars = BTreeMap::new();
        // Would be a hard error under `strict` if the fence were treated as text.
        let out = render_template("```\n{{ missing }}\n```\n", &vars, true).unwrap();
        assert_eq!(out, "```\n{{ missing }}\n```\n");
    }

    #[test]
    fn code_fence_tolerates_phase_two_and_three_syntax() {
        let vars = BTreeMap::new();
        // Statements/filters are rejected outside a fence but pass through untouched inside one.
        let content = "```jinja\n{% if x %}{{ y | upper }}{% endif %}\n```\n";
        let out = render_template(content, &vars, false).unwrap();
        assert_eq!(out, content);
    }

    #[test]
    fn tilde_fence_is_also_protected() {
        let vars = BTreeMap::new();
        let content = "~~~\n{{ missing }}\n~~~\n";
        let out = render_template(content, &vars, false).unwrap();
        assert_eq!(out, content);
    }

    #[test]
    fn unterminated_fence_runs_to_end_of_file() {
        let mut vars = BTreeMap::new();
        vars.insert("community".to_string(), "OpenEHR".to_string());
        let content = "Before {{ community }}\n```\n{{ community }} stays literal\n";
        let out = render_template(content, &vars, false).unwrap();
        assert_eq!(out, "Before OpenEHR\n```\n{{ community }} stays literal\n");
    }

    #[test]
    fn closing_fence_needs_at_least_as_many_delimiters() {
        // A `````` (4 backticks) opener isn't closed by ``` (3); it should run to EOF.
        let vars = BTreeMap::new();
        let content = "````\n{{ missing }}\n```\nstill inside\n````\n";
        let out = render_template(content, &vars, false).unwrap();
        assert_eq!(out, content);
    }

    #[test]
    fn text_around_a_fence_still_renders() {
        let mut vars = BTreeMap::new();
        vars.insert("community".to_string(), "OpenEHR".to_string());
        let content = "Hi {{ community }}\n```\nraw\n```\nBye {{ community }}\n";
        let out = render_template(content, &vars, false).unwrap();
        assert_eq!(out, "Hi OpenEHR\n```\nraw\n```\nBye OpenEHR\n");
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

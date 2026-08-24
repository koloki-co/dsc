// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

mod common;
use common::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn renders_built_in_and_configured_variables() {
    vprintln("e2e_render: rendering a template with built-in and configured variables");
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        r#"[[discourse]]
name = "openehr"
baseurl = "https://discourse.openehr.org"
fullname = "openEHR International"

[template.vars]
organisation = "Koloki Ltd"
community = "Koloki Community"
"#,
    );

    let template_path = dir.path().join("welcome.md");
    fs::write(
        &template_path,
        "Welcome to {{ community }}, brought to you by {{ organisation }}!\nVisit {{ forum_baseurl }} ({{ forum_fullname }}).\n",
    )
    .expect("write template");

    let output = run_dsc(
        &["render", "openehr", template_path.to_str().unwrap()],
        &config_path,
    );
    assert!(
        output.status.success(),
        "render failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Welcome to Koloki Community, brought to you by Koloki Ltd!"));
    assert!(stdout.contains("Visit https://discourse.openehr.org (openEHR International)."));
}

#[test]
fn per_forum_template_vars_override_globals_and_unknown_vars_become_empty() {
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        r#"[[discourse]]
name = "openehr"
baseurl = "https://discourse.openehr.org"

[discourse.template]
organisation = "openEHR International"

[template.vars]
organisation = "Koloki Ltd"
"#,
    );

    let template_path = dir.path().join("notice.md");
    fs::write(
        &template_path,
        "From {{ organisation }}. Contact {{ support_email }}.",
    )
    .expect("write template");

    let output = run_dsc(
        &["render", "openehr", template_path.to_str().unwrap()],
        &config_path,
    );
    assert!(
        output.status.success(),
        "render failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("From openEHR International."));
    assert!(stdout.contains("Contact ."));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("support_email"));
}

#[test]
fn warns_when_template_config_overrides_a_built_in_variable() {
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        r#"[[discourse]]
name = "local"
baseurl = "https://example.com"

[template.vars]
forum_baseurl = "https://configured.example.com"
"#,
    );
    let template_path = dir.path().join("t.md");
    fs::write(&template_path, "Base: {{ forum_baseurl }}").expect("write template");

    let output = run_dsc(
        &["render", "local", template_path.to_str().unwrap()],
        &config_path,
    );
    assert!(output.status.success(), "render failed");
    assert!(String::from_utf8_lossy(&output.stdout).contains("https://configured.example.com"));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("overrides reserved built-in template variable 'forum_baseurl'")
    );
}

#[test]
fn json_format_wraps_rendered_output() {
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        "[[discourse]]\nname = \"local\"\nbaseurl = \"https://example.com\"\n",
    );
    let template_path = dir.path().join("t.md");
    fs::write(&template_path, "Base: {{ forum_baseurl }}").expect("write template");

    let output = run_dsc(
        &[
            "render",
            "local",
            template_path.to_str().unwrap(),
            "-f",
            "json",
        ],
        &config_path,
    );
    assert!(output.status.success(), "render failed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).expect("valid json");
    assert_eq!(parsed["rendered"], "Base: https://example.com");
}

#[test]
fn strict_fails_and_names_every_unknown_variable() {
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        "[[discourse]]\nname = \"local\"\nbaseurl = \"https://example.com\"\n",
    );
    let template_path = dir.path().join("t.md");
    fs::write(
        &template_path,
        "{{ forum_baseurl }} {{ community }} {{ support_email }}",
    )
    .expect("write template");

    let output = run_dsc(
        &[
            "render",
            "local",
            template_path.to_str().unwrap(),
            "--strict",
        ],
        &config_path,
    );
    assert!(!output.status.success(), "strict render should have failed");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("community"), "stderr: {}", stderr);
    assert!(stderr.contains("support_email"), "stderr: {}", stderr);
}

#[test]
fn strict_succeeds_when_every_variable_resolves() {
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        "[[discourse]]\nname = \"local\"\nbaseurl = \"https://example.com\"\n",
    );
    let template_path = dir.path().join("t.md");
    fs::write(&template_path, "Base: {{ forum_baseurl }}").expect("write template");

    let output = run_dsc(
        &[
            "render",
            "local",
            template_path.to_str().unwrap(),
            "--strict",
        ],
        &config_path,
    );
    assert!(
        output.status.success(),
        "strict render failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Base: https://example.com"));
}

#[test]
fn list_vars_prints_the_resolved_map_without_a_file() {
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        r#"[[discourse]]
name = "openehr"
baseurl = "https://discourse.openehr.org"
fullname = "openEHR International"

[discourse.template]
organisation = "openEHR International"

[template.vars]
organisation = "Koloki Ltd"
community = "Koloki Community"
"#,
    );

    let output = run_dsc(&["render", "openehr", "--list-vars"], &config_path);
    assert!(
        output.status.success(),
        "list-vars failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("forum_baseurl = https://discourse.openehr.org"));
    assert!(stdout.contains("forum_name = openehr"));
    assert!(stdout.contains("forum_fullname = openEHR International"));
    assert!(stdout.contains("community = Koloki Community"));
    assert!(stdout.contains("organisation = openEHR International"));
    assert!(!stdout.contains("organisation = Koloki Ltd"));
}

#[test]
fn list_vars_json_emits_the_variable_map() {
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        "[[discourse]]\nname = \"local\"\nbaseurl = \"https://example.com\"\n",
    );

    let output = run_dsc(
        &["render", "local", "--list-vars", "-f", "json"],
        &config_path,
    );
    assert!(output.status.success(), "list-vars json failed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).expect("valid json");
    assert_eq!(parsed["forum_baseurl"], "https://example.com");
    assert_eq!(parsed["forum_name"], "local");
    assert_eq!(parsed["forum_fullname"], "");
}

#[test]
fn render_without_a_file_or_list_vars_is_a_usage_error() {
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        "[[discourse]]\nname = \"local\"\nbaseurl = \"https://example.com\"\n",
    );

    let output = run_dsc(&["render", "local"], &config_path);
    assert!(!output.status.success(), "expected a usage error");
}

#[test]
fn output_flag_writes_to_file() {
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        "[[discourse]]\nname = \"local\"\nbaseurl = \"https://example.com\"\n",
    );
    let template_path = dir.path().join("t.md");
    fs::write(&template_path, "Base: {{ forum_baseurl }}").expect("write template");
    let out_path = dir.path().join("t.rendered.md");

    let output = run_dsc(
        &[
            "render",
            "local",
            template_path.to_str().unwrap(),
            "-o",
            out_path.to_str().unwrap(),
        ],
        &config_path,
    );
    assert!(output.status.success(), "render failed");
    let written = fs::read_to_string(&out_path).expect("read output file");
    assert_eq!(written, "Base: https://example.com");
}

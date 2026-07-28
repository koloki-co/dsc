// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

mod common;
use common::*;
use tempfile::TempDir;

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn plugin_list() {
    let Some(test) = test_discourse() else {
        return;
    };
    vprintln("e2e_plugin_list: listing plugins");
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n",
            test.name, test.baseurl, test.apikey, test.api_username
        ),
    );
    let output = run_dsc(&["plugin", "list", &test.name], &config_path);
    assert!(output.status.success(), "plugin list failed");
}

#[test]
fn plugin_install_remove() {
    const PLUGIN_URL: &str = "https://example.invalid/test-plugin.git";
    const PLUGIN_NAME: &str = "test-plugin";

    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        r#"[[discourse]]
name = "example"
baseurl = "https://example.invalid"
apikey = "fake-api-key"
api_username = "fake-api-user"
ssh_host = "ssh.example.invalid"
"#,
    );

    // SAFETY: this integration-test process has only one non-ignored test, and
    // both variables are set before it spawns any child process.
    unsafe {
        std::env::set_var("DSC_SSH_PLUGIN_INSTALL_CMD", "echo plugin install {url}");
        std::env::set_var("DSC_SSH_PLUGIN_REMOVE_CMD", "echo plugin remove {name}");
    }

    let output = run_dsc(
        &["-n", "plugin", "install", "example", PLUGIN_URL],
        &config_path,
    );
    assert!(
        output.status.success(),
        "plugin install --dry-run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("[dry-run]")
            && stdout.contains("ssh.example.invalid")
            && stdout.contains(PLUGIN_URL),
        "expected plugin install dry-run preview, got: {stdout}"
    );

    let output = run_dsc(
        &["-n", "plugin", "remove", "example", PLUGIN_NAME],
        &config_path,
    );
    assert!(
        output.status.success(),
        "plugin remove --dry-run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("[dry-run]")
            && stdout.contains("ssh.example.invalid")
            && stdout.contains(PLUGIN_NAME),
        "expected plugin remove dry-run preview, got: {stdout}"
    );
}

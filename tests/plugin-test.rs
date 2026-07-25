// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

mod common;
use common::*;
use std::process::Command;
use tempfile::TempDir;

#[test]
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
    let Some(test) = test_discourse() else {
        return;
    };
    if test.ssh_enabled != Some(true) {
        return;
    }
    let Some(url) = test.test_plugin_url.as_ref() else {
        return;
    };
    let Some(name) = test.test_plugin_name.as_ref() else {
        return;
    };
    vprintln("e2e_plugin_install_remove: install/remove plugin");
    let ssh_host_line = test
        .ssh_host
        .as_ref()
        .map(|host| format!("ssh_host = \"{}\"\n", host))
        .unwrap_or_default();
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n{}",
            test.name, test.baseurl, test.apikey, test.api_username, ssh_host_line
        ),
    );

    let output = Command::new(env!("CARGO_BIN_EXE_dsc"))
        .arg("-c")
        .arg(&config_path)
        .arg("plugin")
        .arg("install")
        .arg(&test.name)
        .arg(url)
        .env("DSC_SSH_PLUGIN_INSTALL_CMD", "echo plugin install {url}")
        .output()
        .expect("run plugin install");
    assert!(output.status.success(), "plugin install failed");

    let output = Command::new(env!("CARGO_BIN_EXE_dsc"))
        .arg("-c")
        .arg(&config_path)
        .arg("plugin")
        .arg("remove")
        .arg(&test.name)
        .arg(name)
        .env("DSC_SSH_PLUGIN_REMOVE_CMD", "echo plugin remove {name}")
        .output()
        .expect("run plugin remove");
    assert!(output.status.success(), "plugin remove failed");
}

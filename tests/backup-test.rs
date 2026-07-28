// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

mod common;
use common::*;
use tempfile::TempDir;

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn backup_list() {
    let Some(test) = test_discourse() else {
        return;
    };
    if test.backup_enabled != Some(true) {
        eprintln!("[live:skip] backup_list requires backup_enabled = true");
        return;
    }
    vprintln("e2e_backup_list: listing backups");
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n",
            test.name, test.baseurl, test.apikey, test.api_username
        ),
    );
    let output = run_dsc(&["backup", "list", &test.name], &config_path);
    assert!(output.status.success(), "backup list failed");
}

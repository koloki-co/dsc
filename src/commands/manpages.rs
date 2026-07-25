// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! Man page generation for `dsc` and every subcommand.
//!
//! Mirrors the [`completions`] pattern: introspects the live `clap::Command`
//! tree at runtime and writes one ROFF file per (sub)command. Distro
//! packagers run this once at build time and install the output into
//! section 1 of the man path.
//!
//! [`completions`]: crate::commands::completions

use crate::cli::Cli;
use crate::utils::ensure_dir;
use anyhow::{Context, Result};
use clap::{Command, CommandFactory};
use std::fs;
use std::path::Path;

/// Walk the full subcommand tree and emit one man page per node.
///
/// Naming follows the man-page convention used by `git`, `cargo`, and
/// friends: nested subcommands are joined with hyphens
/// (`dsc-tag-pull.1`). Section is always `1` (user commands).
pub fn write_manpages(dir: &Path) -> Result<()> {
    ensure_dir(dir)?;
    let cmd = Cli::command();
    let mut count: usize = 0;
    write_subtree(&cmd, dir, "", &mut count)?;
    println!("{} man page(s) written to: {}", count, dir.display());
    Ok(())
}

/// Recursively render `cmd` and each of its subcommands into `dir`.
///
/// `prefix` is the hyphen-joined parent name chain (empty for the root
/// command). The root is written as `<bin-name>.1`; nested commands
/// concatenate their names (`dsc-tag-pull.1`).
fn write_subtree(cmd: &Command, dir: &Path, prefix: &str, count: &mut usize) -> Result<()> {
    let leaf_name = cmd.get_name();
    let file_stem = if prefix.is_empty() {
        leaf_name.to_string()
    } else {
        format!("{}-{}", prefix, leaf_name)
    };
    let file_path = dir.join(format!("{}.1", file_stem));

    let man = clap_mangen::Man::new(cmd.clone()).title(file_stem.to_uppercase());
    let mut buffer: Vec<u8> = Vec::new();
    man.render(&mut buffer)
        .with_context(|| format!("rendering {}", file_path.display()))?;
    fs::write(&file_path, &buffer).with_context(|| format!("writing {}", file_path.display()))?;
    *count += 1;

    for sub in cmd.get_subcommands() {
        // Skip the auto-generated `help` subcommand - clap synthesises
        // it for every command and it adds noise to the man-page tree.
        if sub.get_name() == "help" {
            continue;
        }
        write_subtree(sub, dir, &file_stem, count)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::write_manpages;

    #[test]
    fn writes_root_and_subcommand_pages() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_manpages(dir.path()).expect("write_manpages");

        // Root page is always present.
        assert!(dir.path().join("dsc.1").exists());

        // A few representative top-level commands.
        assert!(dir.path().join("dsc-tag.1").exists());
        assert!(dir.path().join("dsc-setting.1").exists());
        assert!(dir.path().join("dsc-config.1").exists());

        // A nested subcommand.
        assert!(dir.path().join("dsc-tag-pull.1").exists());
        assert!(dir.path().join("dsc-setting-diff.1").exists());

        // Auto-generated `help` subcommand is excluded.
        assert!(!dir.path().join("dsc-help.1").exists());
    }

    #[test]
    fn root_page_contains_command_name_and_synopsis() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_manpages(dir.path()).unwrap();
        let body = std::fs::read_to_string(dir.path().join("dsc.1")).unwrap();
        // ROFF .TH header carries the upper-cased command name.
        assert!(body.contains(".TH"));
        assert!(body.contains("DSC"));
        // SYNOPSIS section is always present in clap_mangen output.
        assert!(body.contains("SYNOPSIS"));
    }
}

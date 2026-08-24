// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::cli::{Cli, CompletionCommand, CompletionShell};
use crate::utils::ensure_dir;
use anyhow::{Context, Result, anyhow};
use clap::CommandFactory;
use clap_complete::{Shell, generate};
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub fn run(
    command: Option<CompletionCommand>,
    shell: Option<CompletionShell>,
    dir: Option<&Path>,
) -> Result<()> {
    match command {
        Some(CompletionCommand::Install { shell, dir }) => {
            let shell = shell.or_else(detect_shell).ok_or_else(|| {
                anyhow!("could not detect shell; pass --shell bash|zsh|fish|powershell")
            })?;
            let dir = dir
                .map(Ok)
                .unwrap_or_else(|| default_completion_dir(shell))?;
            write_completions(shell, Some(&dir))?;
            print_install_note(shell, &dir);
            Ok(())
        }
        None => {
            let shell =
                shell.ok_or_else(|| anyhow!("missing shell; try `dsc completions install`"))?;
            write_completions(shell, dir)
        }
    }
}

pub fn write_completions(shell: CompletionShell, dir: Option<&Path>) -> Result<()> {
    let mut cmd = Cli::command();
    let name = cmd.get_name().to_string();
    match dir {
        Some(dir) => {
            ensure_dir(dir)?;
            let filename = completion_filename(shell);
            let path = dir.join(filename);
            let generator: Shell = shell.into();
            if matches!(shell, CompletionShell::Zsh) {
                let mut buffer = Vec::new();
                generate(generator, &mut cmd, name, &mut buffer);
                let content = String::from_utf8(buffer).context("decoding zsh completions")?;
                let content = inject_zsh_sort_style(content);
                let content = inject_zsh_dynamic_discourse_completion(content);
                fs::write(&path, content).with_context(|| format!("writing {}", path.display()))?;
            } else {
                let mut file = fs::File::create(&path)
                    .with_context(|| format!("creating {}", path.display()))?;
                generate(generator, &mut cmd, name, &mut file);
            }
            println!("Completion script written to: {}", path.display());
        }
        None => {
            let generator: Shell = shell.into();
            if matches!(shell, CompletionShell::Zsh) {
                let mut buffer = Vec::new();
                generate(generator, &mut cmd, name, &mut buffer);
                let content = String::from_utf8(buffer).context("decoding zsh completions")?;
                let content = inject_zsh_sort_style(content);
                let content = inject_zsh_dynamic_discourse_completion(content);
                print!("{}", content);
            } else {
                let mut stdout = io::stdout();
                generate(generator, &mut cmd, name, &mut stdout);
            }
        }
    }
    Ok(())
}

fn completion_filename(shell: CompletionShell) -> &'static str {
    match shell {
        CompletionShell::Bash => "dsc",
        CompletionShell::Zsh => "_dsc",
        CompletionShell::Fish => "dsc.fish",
        CompletionShell::PowerShell => "dsc.ps1",
    }
}

fn detect_shell() -> Option<CompletionShell> {
    let shell = env::var("SHELL").ok()?;
    let name = Path::new(&shell).file_name()?.to_string_lossy();
    match name.as_ref() {
        "bash" => Some(CompletionShell::Bash),
        "zsh" => Some(CompletionShell::Zsh),
        "fish" => Some(CompletionShell::Fish),
        _ => None,
    }
}

fn default_completion_dir(shell: CompletionShell) -> Result<PathBuf> {
    let home = home_dir().ok_or_else(|| anyhow!("could not determine home directory"))?;
    Ok(match shell {
        CompletionShell::Bash => env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"))
            .join("bash-completion/completions"),
        CompletionShell::Zsh => home.join(".zfunc"),
        CompletionShell::Fish => env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"))
            .join("fish/completions"),
        CompletionShell::PowerShell => home.join(".config/powershell/completions"),
    })
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn print_install_note(shell: CompletionShell, dir: &Path) {
    match shell {
        CompletionShell::Zsh => {
            println!("Add this before `compinit` in ~/.zshrc if it is not already there:");
            println!("  fpath=({} $fpath)", dir.display());
            println!("Then restart zsh or run `autoload -Uz compinit && compinit`.");
        }
        CompletionShell::PowerShell => {
            println!("Add this to your PowerShell profile if it is not already there:");
            println!("  . {}/dsc.ps1", dir.display());
        }
        _ => println!("Restart your shell to load the updated completions."),
    }
}

fn inject_zsh_sort_style(mut content: String) -> String {
    let style = "zstyle ':completion:*:dsc:*' sort false";
    if content.contains(style) {
        return content;
    }
    let marker = "autoload -U is-at-least\n";
    if let Some(pos) = content.find(marker) {
        let insert_at = pos + marker.len();
        content.insert_str(insert_at, &format!("\n{}\n", style));
        return content;
    }
    format!("{}\n\n{}", style, content)
}

fn inject_zsh_dynamic_discourse_completion(mut content: String) -> String {
    if !content.contains("_dsc_discourse_names()") {
        let marker = "autoload -U is-at-least\n";
        let function = "\n_dsc_discourse_names() {\n\
    local config_path\n\
    local i\n\
    for i in {1..$#words}; do\n\
        if [[ ${words[$i]} == -c || ${words[$i]} == --config ]]; then\n\
            config_path=${words[$((i+1))]}\n\
        elif [[ ${words[$i]} == --config=* ]]; then\n\
            config_path=${words[$i]#--config=}\n\
        fi\n\
    done\n\
\n\
    local cmd=(dsc list --format plaintext)\n\
    if [[ -n ${config_path:-} ]]; then\n\
        cmd+=(-c \"$config_path\")\n\
    fi\n\
\n\
    local -a names\n\
    names=(${(f)\"$(command ${cmd[@]} 2>/dev/null | sed 's/ - .*//')\"})\n\
    _describe -t discourses 'discourses' names\n\
}\n";
        if let Some(pos) = content.find(marker) {
            let insert_at = pos + marker.len();
            content.insert_str(insert_at, function);
        } else {
            content = format!("{}{}", function.trim_start(), content);
        }
    }

    replace_discourse_arg_completion(content)
}

/// Replace `:_default` with `:_dsc_discourse_names` for all required and
/// optional `discourse` positional args across every subcommand.
fn replace_discourse_arg_completion(content: String) -> String {
    // Clap emits `:discourse` for required positionals and `::discourse` for
    // optional ones. Replacing the generated completion here keeps candidates
    // dynamic without hand-maintaining the command tree.
    let mut result = String::with_capacity(content.len());
    let mut remaining = content.as_str();

    while let Some(pos) = remaining.find("'") {
        result.push_str(&remaining[..pos]);
        let after = &remaining[pos..];
        let argument_prefix = if after.starts_with("':discourse") {
            "':discourse"
        } else if after.starts_with("'::discourse") {
            "'::discourse"
        } else {
            result.push('\'');
            remaining = &after[1..];
            continue;
        };

        if let Some(default_pos) = after.find(":_default'") {
            result.push_str(&after[..default_pos]);
            result.push_str(":_dsc_discourse_names'");
            remaining = &after[default_pos + ":_default'".len()..];
        } else {
            result.push_str(&after[..argument_prefix.len()]);
            remaining = &after[argument_prefix.len()..];
        }
    }
    result.push_str(remaining);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zsh_install_defaults_to_zfunc() {
        let dir = default_completion_dir(CompletionShell::Zsh)
            .expect("test environment has a home directory");

        assert!(dir.ends_with(".zfunc"));
    }
}

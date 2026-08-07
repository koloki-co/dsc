# Shell completions

Install or generate shell completion scripts for bash, zsh, fish, or PowerShell.

```
dsc completions <shell> [--dir <path>]
dsc completions install [--shell <shell>] [--dir <path>]
```

If `--dir` is provided, writes the completion script to the given directory. Otherwise, prints to stdout.

## Setup

For the current user, prefer the installer:

```bash
dsc completions install
```

It detects your shell, writes the correctly named completion file, and prints any one-time shell setup still needed. Re-run it after upgrading `dsc` if your package manager or installer did not refresh completions for you.

For packaging or custom installs:

```bash
# Bash
dsc completions --dir /usr/local/share/bash-completion/completions bash

# Zsh
dsc completions --dir ~/.zfunc zsh
# Ensure this is in ~/.zshrc before compinit, if it is not already:
fpath=(~/.zfunc $fpath)
autoload -Uz compinit && compinit

# Fish
dsc completions --dir ~/.config/fish/completions fish

# PowerShell
dsc completions --dir ~/.config/powershell/completions powershell
```

The Zsh generator writes a `_dsc` file in the target directory. It includes dynamic completion of discourse names for positional arguments (calls `dsc list --format plaintext` at completion time).

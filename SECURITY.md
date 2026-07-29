# Security policy

## Reporting a vulnerability

Please **don't** open a public GitHub issue for security problems.

Use one of:

- **GitHub Security Advisories** — preferred. Open a draft advisory at <https://github.com/koloki-co/dsc/security/advisories/new>. This stays private until coordinated disclosure.
- **Email** — `info@koloki.co` for issues that don't fit the GitHub flow.

Please include:

- The dsc version (`dsc version`).
- A clear description of the issue and its impact.
- Reproduction steps if possible.
- Whether you've found a fix or workaround.

You should expect an initial reply within a few business days. Public disclosure happens only after a fix is available, with credit to the reporter unless they prefer otherwise.

## Scope

dsc runs in two modes that have very different security profiles:

### 1. Discourse API client (most commands)

`dsc list`, `dsc topic *`, `dsc user *`, `dsc post *`, etc. all act as a Discourse API client using the `apikey` / `api_username` from `dsc.toml`. The risks here are mostly around credential handling and accidental destructive operations. Notable:

- `dsc.toml` typically contains admin API keys. Treat it like `~/.ssh/config`.
- Destructive operations (`backup restore`, `setting set`, `topic push`, `topic new`, `post delete`, plugin/theme install/remove, `category copy`, `group copy`, etc.) all honour `--dry-run`.
- 429 retries with sane defaults. Bug-class concern would be loops or credential leakage in error output.

### 2. Server-level operations (`dsc update`, `dsc harden`, `dsc install`)

Run shell commands as root (`update`) or as root-then-non-root (`harden`, `install`) over SSH. Bug-class concerns are higher: shell injection, dropping bad sudoers entries, breaking sshd, etc. We treat issues in these commands as security-class bugs by default — please report rather than self-fix and PR.

`dsc harden` in particular publishes a hardening blueprint; see [docs/harden.md](docs/harden.md#why-publish-a-hardening-routine) for the rationale and mitigations.

## What's out of scope

- Bugs in Discourse itself, the Discourse REST API, or `discourse_docker`. Report those upstream at <https://meta.discourse.org>.
- Bugs in cargo-dist, the Homebrew tap publishing, or other third-party packaging tools. Report upstream.
- Issues with users putting their own API keys in source control or world-readable files — that's an operator concern, not a dsc bug.

## Receiving security updates

The project follows a "fix, then announce" model. Where a vulnerability requires action by users:

1. The patched version is released to crates.io, GitHub Releases, and the Homebrew tap.
2. A short advisory is published on GitHub Security Advisories with a CVSS score.
3. Major issues additionally get a brief note on [Discourse Meta](https://meta.discourse.org) once the project announcement thread exists.

For advance notice of advisories before they go public, watch the repo for "Security advisories" notifications (Settings → Notifications → Custom → Security advisories).

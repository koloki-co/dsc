// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! Read-only Docker daemon preflight for `dsc update` (roadmap R81).
//!
//! `dsc update` picks rootful or rootless Docker from `docker_rootless` in
//! `dsc.toml` alone. On a host where the other daemon also exists - or where
//! the `app` container actually lives under the other daemon - that choice
//! can rebuild a second `app` against the same shared data. This module runs
//! one read-only SSH probe before any disruptive step, classifies what it
//! found, and either warns or refuses. It never starts, stops, enables,
//! disables, or migrates a daemon or container, and never changes ownership.
//!
//! Inability to inspect a daemon is always reported as "unknown", never as
//! absence. See spec/commands/update-daemon-preflight.md.

/// The single read-only probe. Each line is `KEY=VALUE`; unknown lines are
/// ignored by the parser. Every `docker` call names its socket explicitly so
/// an inherited `DOCKER_HOST` cannot make both probes hit the same daemon.
///
/// - `ROOTFUL_ENABLED` / `ROOTFUL_ACTIVE`: `systemctl is-enabled|is-active`
///   for the system `docker.service` and `docker.socket` (any `enabled` or
///   `active` wins). A disabled, inactive unit is what Docker's own rootless
///   setup leaves behind from the `docker-ce` package, so it does not count
///   as a rootful installation in use.
/// - `ROOTLESS_UNIT`: whether the SSH user has a user `docker.service`.
/// - `ROOTLESS_ACTIVE`: `systemctl --user is-active docker.service`.
/// - `ROOTFUL_APP` / `ROOTLESS_APP`: the `app` container's state under that
///   daemon, `none` when it has no `app`, `?` when the query failed.
/// - `ROOTLESSKIT_USERS`: other users running `rootlesskit`, which reveals a
///   rootless daemon owned by someone other than the SSH user.
pub(crate) const DAEMON_PROBE_CMD: &str = r#"st() { s=$(systemctl "$@" 2>/dev/null | head -n1); echo "${s:-unknown}"; }
echo "ROOTFUL_ENABLED=$(st is-enabled docker.service) $(st is-enabled docker.socket)"
echo "ROOTFUL_ACTIVE=$(st is-active docker.service) $(st is-active docker.socket)"
if [ -f "$HOME/.config/systemd/user/docker.service" ] || systemctl --user cat docker.service >/dev/null 2>&1; then echo ROOTLESS_UNIT=yes; else echo ROOTLESS_UNIT=no; fi
echo "ROOTLESS_ACTIVE=$(st --user is-active docker.service)"
if a=$(sudo -n docker -H unix:///var/run/docker.sock ps -a --filter 'name=^app$' --format '{{.State}}' 2>/dev/null); then echo "ROOTFUL_APP=${a:-none}"; else echo "ROOTFUL_APP=?"; fi
if a=$(docker -H "unix://${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/docker.sock" ps -a --filter 'name=^app$' --format '{{.State}}' 2>/dev/null); then echo "ROOTLESS_APP=${a:-none}"; else echo "ROOTLESS_APP=?"; fi
echo "ROOTLESSKIT_USERS=$(ps -eo user=,comm= 2>/dev/null | awk -v me="$(id -un)" '$2=="rootlesskit" && $1!=me {print $1}' | sort -u | tr '\n' ' ')"
"#;

/// Whether a daemon is present, as far as the probe could tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Presence {
    /// Enabled or running.
    Active,
    /// Installed (enabled, or a user unit exists) but not running.
    Inactive,
    /// No sign of an installation in use.
    Absent,
    /// The probe could not tell.
    Unknown,
}

/// The `app` container under one daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AppState {
    /// No `app` container on this daemon.
    None,
    /// Docker's lower-cased state (`running`, `exited`, `restarting`, ...).
    State(String),
    /// The daemon could not be queried (inactive, no access, no CLI).
    Unknown,
}

impl AppState {
    fn is_live(&self) -> bool {
        matches!(self, AppState::State(s) if matches!(s.as_str(), "running" | "restarting"))
    }

    fn exists(&self) -> bool {
        matches!(self, AppState::State(_))
    }

    fn describe(&self) -> String {
        match self {
            AppState::None => "no app container".to_string(),
            AppState::State(s) => format!("app {s}"),
            AppState::Unknown => "app state unknown (daemon could not be queried)".to_string(),
        }
    }
}

/// One daemon's probe result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DaemonProbe {
    pub presence: Presence,
    pub app: AppState,
}

/// The parsed probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HostDaemons {
    pub rootful: DaemonProbe,
    pub rootless: DaemonProbe,
    /// Users other than the SSH user running `rootlesskit`.
    pub other_rootless_users: Vec<String>,
}

/// What the update should do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DaemonVerdict {
    /// Messages to print; the update continues.
    pub warnings: Vec<String>,
    /// When set, the update must stop before any disruptive step.
    pub refusal: Option<String>,
}

fn label(rootless: bool) -> &'static str {
    if rootless { "rootless" } else { "rootful" }
}

fn presence_label(p: Presence) -> &'static str {
    match p {
        Presence::Active => "active",
        Presence::Inactive => "installed but inactive",
        Presence::Absent => "absent",
        Presence::Unknown => "unknown",
    }
}

fn parse_app(value: Option<&str>) -> AppState {
    match value.map(str::trim) {
        None | Some("?") | Some("") => AppState::Unknown,
        Some("none") => AppState::None,
        Some(state) => {
            // `docker ps` prints one line per match; `name=^app$` matches at
            // most one, but only the first token is ever trusted.
            let first = state.split_whitespace().next().unwrap_or("");
            AppState::State(first.to_ascii_lowercase())
        }
    }
}

fn words(value: Option<&str>) -> Vec<&str> {
    value
        .map(|v| v.split_whitespace().collect())
        .unwrap_or_default()
}

/// Parse [`DAEMON_PROBE_CMD`] output. Missing keys become `Unknown`.
pub(crate) fn parse_daemon_probe(output: &str) -> HostDaemons {
    let mut fields = std::collections::HashMap::new();
    for line in output.lines() {
        if let Some((key, value)) = line.trim().split_once('=') {
            fields.entry(key.trim()).or_insert(value);
        }
    }
    let get = |key: &str| fields.get(key).copied();

    let rootful_enabled = words(get("ROOTFUL_ENABLED"));
    let rootful_active = words(get("ROOTFUL_ACTIVE"));
    let rootful_presence = if rootful_active.contains(&"active") {
        Presence::Active
    } else if rootful_enabled
        .iter()
        .any(|s| matches!(*s, "enabled" | "enabled-runtime"))
    {
        Presence::Inactive
    } else if rootful_enabled.is_empty()
        || rootful_active.is_empty()
        || rootful_enabled.iter().all(|s| *s == "unknown")
            && rootful_active.iter().all(|s| *s == "unknown")
    {
        Presence::Unknown
    } else {
        Presence::Absent
    };

    let rootless_active = get("ROOTLESS_ACTIVE").map(str::trim);
    let rootless_presence = match (get("ROOTLESS_UNIT").map(str::trim), rootless_active) {
        (_, Some("active")) => Presence::Active,
        (Some("yes"), _) => Presence::Inactive,
        (Some("no"), _) => Presence::Absent,
        _ => Presence::Unknown,
    };

    let mut rootful_app = parse_app(get("ROOTFUL_APP"));
    let mut rootless_app = parse_app(get("ROOTLESS_APP"));
    // A daemon with no sign of installation cannot be queried, so its
    // failed `docker ps` means "no app here", not "unknown".
    if rootful_presence == Presence::Absent && rootful_app == AppState::Unknown {
        rootful_app = AppState::None;
    }
    if rootless_presence == Presence::Absent && rootless_app == AppState::Unknown {
        rootless_app = AppState::None;
    }

    let other_rootless_users = words(get("ROOTLESSKIT_USERS"))
        .into_iter()
        .map(str::to_string)
        .collect();

    HostDaemons {
        rootful: DaemonProbe {
            presence: rootful_presence,
            app: rootful_app,
        },
        rootless: DaemonProbe {
            presence: rootless_presence,
            app: rootless_app,
        },
        other_rootless_users,
    }
}

/// Decide whether an update that selects `rootless` may proceed.
pub(crate) fn classify_daemons(host: &HostDaemons, rootless: bool) -> DaemonVerdict {
    let (selected, other) = if rootless {
        (&host.rootless, &host.rootful)
    } else {
        (&host.rootful, &host.rootless)
    };
    let sel = label(rootless);
    let oth = label(!rootless);
    let config_hint = format!(
        "dsc.toml selects {sel} Docker (docker_rootless = {rootless}{})",
        if rootless { "" } else { ", or unset" }
    );
    let mut warnings = Vec::new();

    let installed = |p: Presence| matches!(p, Presence::Active | Presence::Inactive);
    if installed(host.rootful.presence) && installed(host.rootless.presence) {
        warnings.push(format!(
            "both rootful ({}) and rootless ({}) Docker are installed on this host; a Discourse host normally has one, so review this before relying on the update",
            presence_label(host.rootful.presence),
            presence_label(host.rootless.presence)
        ));
    }
    if !host.other_rootless_users.is_empty() {
        warnings.push(format!(
            "a rootless Docker daemon (rootlesskit) is running for other user(s): {}; this probe cannot see their containers",
            host.other_rootless_users.join(", ")
        ));
    }

    if other.app.is_live() {
        return DaemonVerdict {
            warnings,
            refusal: Some(format!(
                "{config_hint}, but an app container is {} under the {oth} daemon. Rebuilding would start a second app against the same shared data. Stop the unused container or correct docker_rootless, then retry. dsc makes no change to either daemon.",
                match &other.app {
                    AppState::State(s) => s.as_str(),
                    _ => "live",
                }
            )),
        };
    }
    if selected.app == AppState::None && other.app.exists() {
        return DaemonVerdict {
            warnings,
            refusal: Some(format!(
                "{config_hint}, which has no app container, but the {oth} daemon has one ({}). docker_rootless is probably wrong for this host; correct it in dsc.toml, then retry. dsc does not switch daemons automatically.",
                other.app.describe()
            )),
        };
    }
    if other.app.exists() {
        warnings.push(format!(
            "the {oth} daemon also holds an app container ({}); if it is restarted it would run against the same shared data",
            other.app.describe()
        ));
    }
    if selected.app == AppState::Unknown {
        warnings.push(format!(
            "could not inspect the selected {sel} daemon ({}); its app container was not verified",
            presence_label(selected.presence)
        ));
    }
    if installed(other.presence) && other.app == AppState::Unknown {
        warnings.push(format!(
            "could not inspect the {oth} daemon ({}); a duplicate app container there cannot be ruled out",
            presence_label(other.presence)
        ));
    }

    DaemonVerdict {
        warnings,
        refusal: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(lines: &[&str]) -> HostDaemons {
        parse_daemon_probe(&lines.join("\n"))
    }

    const ROOTLESS_ONLY: &[&str] = &[
        "ROOTFUL_ENABLED=disabled disabled",
        "ROOTFUL_ACTIVE=inactive inactive",
        "ROOTLESS_UNIT=yes",
        "ROOTLESS_ACTIVE=active",
        "ROOTFUL_APP=?",
        "ROOTLESS_APP=running",
        "ROOTLESSKIT_USERS=",
    ];

    const ROOTFUL_ONLY: &[&str] = &[
        "ROOTFUL_ENABLED=enabled enabled",
        "ROOTFUL_ACTIVE=active active",
        "ROOTLESS_UNIT=no",
        "ROOTLESS_ACTIVE=inactive",
        "ROOTFUL_APP=running",
        "ROOTLESS_APP=?",
        "ROOTLESSKIT_USERS=",
    ];

    fn with(base: &[&str], key: &str, value: &str) -> Vec<String> {
        base.iter()
            .map(|line| {
                if line.starts_with(&format!("{key}=")) {
                    format!("{key}={value}")
                } else {
                    line.to_string()
                }
            })
            .collect()
    }

    fn probe_owned(lines: &[String]) -> HostDaemons {
        parse_daemon_probe(&lines.join("\n"))
    }

    #[test]
    fn rootless_only_host_is_clean() {
        let host = probe(ROOTLESS_ONLY);
        assert_eq!(host.rootful.presence, Presence::Absent);
        assert_eq!(host.rootful.app, AppState::None);
        assert_eq!(host.rootless.presence, Presence::Active);
        let verdict = classify_daemons(&host, true);
        assert_eq!(verdict.refusal, None);
        assert!(verdict.warnings.is_empty(), "{:?}", verdict.warnings);
    }

    #[test]
    fn rootful_only_host_is_clean() {
        let verdict = classify_daemons(&probe(ROOTFUL_ONLY), false);
        assert_eq!(verdict.refusal, None);
        assert!(verdict.warnings.is_empty(), "{:?}", verdict.warnings);
    }

    #[test]
    fn mixed_installation_warns_even_without_duplicate_containers() {
        // Rootful enabled but stopped, rootless serving: no duplicate app,
        // but a mixed installation is itself worth review.
        let lines = with(ROOTLESS_ONLY, "ROOTFUL_ENABLED", "enabled disabled");
        let verdict = classify_daemons(&probe_owned(&lines), true);
        assert_eq!(verdict.refusal, None);
        assert!(verdict.warnings.iter().any(|w| w.contains("both rootful")));
        // Inability to inspect the installed rootful daemon is not absence.
        assert!(
            verdict
                .warnings
                .iter()
                .any(|w| w.contains("cannot be ruled out"))
        );
    }

    #[test]
    fn disabled_package_unit_is_not_a_rootful_installation() {
        let host = probe(ROOTLESS_ONLY);
        assert_eq!(host.rootful.presence, Presence::Absent);
    }

    #[test]
    fn refuses_when_the_unselected_daemon_has_a_live_app() {
        // The 2026-11-06 field shape: config omits docker_rootless (rootful)
        // while the rootless app is restarting against the same data.
        let mut lines = with(ROOTFUL_ONLY, "ROOTLESS_UNIT", "yes");
        lines = with_owned(&lines, "ROOTLESS_ACTIVE", "active");
        lines = with_owned(&lines, "ROOTLESS_APP", "restarting");
        let verdict = classify_daemons(&probe_owned(&lines), false);
        let refusal = verdict.refusal.expect("must refuse");
        assert!(refusal.contains("restarting under the rootless daemon"));
        assert!(refusal.contains("docker_rootless = false, or unset"));
        assert!(verdict.warnings.iter().any(|w| w.contains("both rootful")));
    }

    #[test]
    fn refuses_when_only_the_unselected_daemon_has_an_app() {
        // The 2026-10-29 trigger: rootless install, config unset, a stopped
        // rootless app and no rootful app.
        let lines = with(ROOTLESS_ONLY, "ROOTLESS_APP", "exited");
        let lines = with_owned(&lines, "ROOTFUL_ENABLED", "enabled enabled");
        let lines = with_owned(&lines, "ROOTFUL_ACTIVE", "active active");
        let lines = with_owned(&lines, "ROOTFUL_APP", "none");
        let verdict = classify_daemons(&probe_owned(&lines), false);
        let refusal = verdict.refusal.expect("must refuse");
        assert!(refusal.contains("has no app container"));
        assert!(refusal.contains("app exited"));
    }

    #[test]
    fn stopped_duplicate_warns_but_proceeds() {
        let lines = with(ROOTFUL_ONLY, "ROOTLESS_UNIT", "yes");
        let lines = with_owned(&lines, "ROOTLESS_APP", "exited");
        let verdict = classify_daemons(&probe_owned(&lines), false);
        assert_eq!(verdict.refusal, None);
        assert!(
            verdict
                .warnings
                .iter()
                .any(|w| w.contains("also holds an app container (app exited)"))
        );
    }

    #[test]
    fn both_daemons_accessible_with_live_apps_refuses_either_way() {
        let lines = with(ROOTFUL_ONLY, "ROOTLESS_UNIT", "yes");
        let lines = with_owned(&lines, "ROOTLESS_ACTIVE", "active");
        let lines = with_owned(&lines, "ROOTLESS_APP", "running");
        let host = probe_owned(&lines);
        assert!(classify_daemons(&host, false).refusal.is_some());
        assert!(classify_daemons(&host, true).refusal.is_some());
    }

    #[test]
    fn inaccessible_selected_daemon_warns_not_absent() {
        let lines = with(ROOTFUL_ONLY, "ROOTFUL_APP", "?");
        let host = probe_owned(&lines);
        assert_eq!(host.rootful.app, AppState::Unknown);
        let verdict = classify_daemons(&host, false);
        assert_eq!(verdict.refusal, None);
        assert!(
            verdict
                .warnings
                .iter()
                .any(|w| w.contains("could not inspect the selected rootful daemon"))
        );
    }

    #[test]
    fn empty_or_garbled_probe_is_unknown_not_absent() {
        let host = parse_daemon_probe("garbage\n");
        assert_eq!(host.rootful.presence, Presence::Unknown);
        assert_eq!(host.rootless.presence, Presence::Unknown);
        assert_eq!(host.rootful.app, AppState::Unknown);
        let verdict = classify_daemons(&host, true);
        assert_eq!(verdict.refusal, None);
        assert!(
            verdict
                .warnings
                .iter()
                .any(|w| w.contains("could not inspect the selected rootless daemon"))
        );
    }

    #[test]
    fn other_users_rootlesskit_is_reported() {
        let lines = with(ROOTFUL_ONLY, "ROOTLESSKIT_USERS", "discourse ");
        let verdict = classify_daemons(&probe_owned(&lines), false);
        assert!(verdict.warnings.iter().any(|w| w.contains("discourse")));
    }

    #[test]
    fn probe_is_read_only_and_names_sockets_explicitly() {
        for forbidden in [
            " start", " stop", " restart", " enable", " disable", " rm", "chown", "chmod",
            "launcher", "prune", "setcap",
        ] {
            assert!(
                !DAEMON_PROBE_CMD.contains(forbidden),
                "probe must stay read-only: found {forbidden:?}"
            );
        }
        assert!(DAEMON_PROBE_CMD.contains("sudo -n docker -H unix:///var/run/docker.sock ps -a"));
        assert!(DAEMON_PROBE_CMD.contains("docker.sock\" ps -a"));
    }

    #[test]
    fn probe_runs_under_a_real_shell() {
        // Executes the probe locally to prove it is valid POSIX sh and always
        // emits every key, whatever this machine's Docker state is.
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(DAEMON_PROBE_CMD)
            .env("PATH", "/usr/bin:/bin")
            .output()
            .expect("run sh");
        let text = String::from_utf8_lossy(&output.stdout);
        for key in [
            "ROOTFUL_ENABLED=",
            "ROOTFUL_ACTIVE=",
            "ROOTLESS_UNIT=",
            "ROOTLESS_ACTIVE=",
            "ROOTFUL_APP=",
            "ROOTLESS_APP=",
            "ROOTLESSKIT_USERS=",
        ] {
            assert!(text.contains(key), "missing {key} in {text}");
        }
    }

    fn with_owned(base: &[String], key: &str, value: &str) -> Vec<String> {
        let refs: Vec<&str> = base.iter().map(String::as_str).collect();
        with(&refs, key, value)
    }
}

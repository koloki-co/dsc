// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::utils::{atomic_write_private, expand_tilde_path, parse_hex_color};
use anyhow::{Context, Result, anyhow};
use serde::de::Deserializer;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

/// Env var pointing at an explicit config file path. Wins over the discovered
/// search hierarchy; missing-file is an error, not a silent fall-through.
pub const ENV_CONFIG: &str = "DSC_CONFIG";

/// Env var pointing at the user config-home directory. Defaults to
/// `$XDG_CONFIG_HOME/dsc`, which itself defaults to `~/.config/dsc`.
/// `dsc` looks for `dsc.toml` inside this directory.
pub const ENV_CONFIG_HOME: &str = "DSC_CONFIG_HOME";

fn deserialize_opt_string_empty_as_none<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    Ok(value.filter(|s| !s.is_empty()))
}

fn deserialize_opt_u64_zero_as_none<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<u64>::deserialize(deserializer)?;
    Ok(value.filter(|v| *v != 0))
}

/// Top-level configuration for dsc.
#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct Config {
    #[serde(default)]
    pub discourse: Vec<DiscourseConfig>,
    #[serde(default)]
    pub harden: HardenConfig,
    #[serde(default)]
    pub template: TemplateConfig,
}

/// `[template]` section of `dsc.toml`: global variables available to
/// `dsc render` across every forum, overridden by a matching
/// `[discourse.template]` key on the target forum. Flat string map only.
#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct TemplateConfig {
    #[serde(default)]
    pub vars: BTreeMap<String, String>,
}

/// User overrides for `dsc harden` defaults. Every field is optional;
/// anything left unset falls back to the built-in defaults applied in
/// `commands::harden::resolve_options`. CLI flags override this block on
/// a per-run basis.
#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct HardenConfig {
    /// Username for the new sudo-enabled non-root account. Default: `discourse`.
    #[serde(default, deserialize_with = "deserialize_opt_string_empty_as_none")]
    pub new_user: Option<String>,
    /// SSH port to move the daemon to in stage 2. Default: 2227.
    #[serde(default, deserialize_with = "deserialize_opt_u64_zero_as_none")]
    pub ssh_port: Option<u64>,
    /// URL to fetch the Docker installer from. Default: `https://get.docker.com`.
    #[serde(default, deserialize_with = "deserialize_opt_string_empty_as_none")]
    pub docker_install_url: Option<String>,
    /// Whether to install Docker rootless. Default: true.
    #[serde(default)]
    pub docker_rootless: Option<bool>,
    /// Swap file size in GB. 0 to skip. Default: 2.
    #[serde(default)]
    pub swap_size_gb: Option<u32>,
    /// Cap on journald disk use. Default: `500M`.
    #[serde(default, deserialize_with = "deserialize_opt_string_empty_as_none")]
    pub journald_max_use: Option<String>,
    /// Timezone to set via `timedatectl`. Default: `UTC`.
    #[serde(default, deserialize_with = "deserialize_opt_string_empty_as_none")]
    pub timezone: Option<String>,
    /// Whether to enable unattended security upgrades. Default: true.
    #[serde(default)]
    pub unattended_security_upgrades: Option<bool>,
    /// Whether to install fail2ban. Default: true.
    #[serde(default)]
    pub fail2ban: Option<bool>,
    /// Whether to install mosh and open UDP 60000-61000. Default: false.
    #[serde(default)]
    pub mosh: Option<bool>,
    /// Override sshd `Ciphers` line. Defaults to dsc's policy overlay
    /// (drop legacy algorithms while preserving upstream defaults).
    #[serde(default, deserialize_with = "deserialize_opt_string_empty_as_none")]
    pub sshd_ciphers: Option<String>,
    /// Override sshd `KexAlgorithms` line. Defaults to dsc's policy overlay
    /// (prefer PQ-hybrid first, disable legacy SHA-1 DH groups).
    #[serde(default, deserialize_with = "deserialize_opt_string_empty_as_none")]
    pub sshd_kex: Option<String>,
    /// Override sshd `MACs` line. Defaults to dsc's policy overlay
    /// (disable legacy SHA-1/MD5 and short UMAC variants).
    #[serde(default, deserialize_with = "deserialize_opt_string_empty_as_none")]
    pub sshd_macs: Option<String>,
    /// Extra ufw `allow` rules applied after the standard set
    /// (e.g. `["3000/tcp", "192.168.1.0/24"]`).
    #[serde(default)]
    pub extra_ufw_allow: Option<Vec<String>>,
}

/// Configuration for a single Discourse install.
#[derive(Serialize, Deserialize, Default, Clone)]
pub struct DiscourseConfig {
    pub name: String,
    pub baseurl: String,
    #[serde(default, deserialize_with = "deserialize_opt_string_empty_as_none")]
    pub fullname: Option<String>,
    #[serde(default, deserialize_with = "deserialize_opt_string_empty_as_none")]
    pub apikey: Option<String>,
    #[serde(default, deserialize_with = "deserialize_opt_string_empty_as_none")]
    pub api_username: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default, deserialize_with = "deserialize_opt_u64_zero_as_none")]
    pub changelog_topic_id: Option<u64>,
    #[serde(default, deserialize_with = "deserialize_opt_string_empty_as_none")]
    pub ssh_host: Option<String>,
    /// Path to the Discourse Docker app configuration on the remote host.
    /// Defaults to `/var/discourse/containers/app.yml` when omitted.
    #[serde(default, deserialize_with = "deserialize_opt_string_empty_as_none")]
    pub app_yml_path: Option<String>,
    #[serde(default)]
    pub docker_rootless: Option<bool>,
    /// Discourse branch to compare against during `dsc update`. Most
    /// Discourse sites run `latest` (which tracks `main` closely); `stable`
    /// is used by sites that prefer less frequent updates. Default: `latest`.
    #[serde(default, deserialize_with = "deserialize_opt_string_empty_as_none")]
    pub discourse_branch: Option<String>,
    /// Cached theme-derived key colour for `dsc update` labels, as a strict
    /// `#RRGGBB` value. A cache of a user-selected colour, not an authority
    /// refreshed on every update; `dsc update` never writes this back. An
    /// unset or invalid value falls back to the deterministic hash colour.
    #[serde(default, deserialize_with = "deserialize_opt_string_empty_as_none")]
    pub update_colour: Option<String>,
    /// Per-forum template variables for `dsc render`, overriding
    /// `[template.vars]` globals of the same name and introducing new ones.
    #[serde(default)]
    pub template: BTreeMap<String, String>,
}

impl fmt::Debug for DiscourseConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DiscourseConfig")
            .field("name", &self.name)
            .field("baseurl", &self.baseurl)
            .field("fullname", &self.fullname)
            .field(
                "apikey",
                &self.apikey.as_ref().map(|_| "#####REDACTED#####"),
            )
            .field("api_username", &self.api_username)
            .field("tags", &self.tags)
            .field("changelog_topic_id", &self.changelog_topic_id)
            .field("ssh_host", &self.ssh_host)
            .field("app_yml_path", &self.app_yml_path)
            .field("docker_rootless", &self.docker_rootless)
            .field("discourse_branch", &self.discourse_branch)
            .field("update_colour", &self.update_colour)
            .field("template", &self.template)
            .finish()
    }
}

/// Load configuration from a TOML file.
pub fn load_config(path: &Path) -> Result<Config> {
    if !path.exists() {
        return Ok(Config::default());
    }
    let raw = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let config: Config = toml::from_str(&raw).with_context(|| "parsing config")?;
    warn_on_discourse_names(&config);
    warn_on_invalid_update_colour(&config);
    Ok(config)
}

/// Save configuration to a TOML file.
pub fn save_config(path: &Path, config: &Config) -> Result<()> {
    let raw = toml::to_string_pretty(config).with_context(|| "serializing config")?;
    write_config_file(path, raw.as_bytes())?;
    Ok(())
}

fn write_config_file(path: &Path, raw: &[u8]) -> Result<()> {
    atomic_write_private(path, raw, true)
}

/// Find a discourse by name.
pub fn find_discourse<'a>(config: &'a Config, name: &str) -> Option<&'a DiscourseConfig> {
    config.discourse.iter().find(|d| d.name == name)
}

/// Find a discourse by name (mutable).
pub fn find_discourse_mut<'a>(
    config: &'a mut Config,
    name: &str,
) -> Option<&'a mut DiscourseConfig> {
    config.discourse.iter_mut().find(|d| d.name == name)
}

fn warn_on_discourse_names(config: &Config) {
    for discourse in &config.discourse {
        if discourse.name.chars().any(|ch| ch.is_whitespace()) {
            eprintln!(
                "Warning: discourse name '{}' contains whitespace. Prefer a short, slugified name without spaces; use 'fullname' for display.",
                discourse.name
            );
        }
    }
}

fn warn_on_invalid_update_colour(config: &Config) {
    for discourse in &config.discourse {
        if let Some(value) = discourse.update_colour.as_deref()
            && parse_hex_color(value).is_none()
        {
            eprintln!(
                "Warning: discourse '{}' has invalid update_colour '{}' (expected '#RRGGBB'); falling back to the default label colour.",
                discourse.name, value
            );
        }
    }
}

/// Where the active config came from. Used by `dsc config` to label the
/// active path so the user understands why a file outside the standard
/// hierarchy is in use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigSource {
    /// Explicit `--config`/`-c` flag.
    Flag(PathBuf),
    /// `$DSC_CONFIG` env var.
    EnvVar(PathBuf),
    /// First existing path from the search hierarchy.
    Discovered(PathBuf),
    /// No file found anywhere; fallback to `./dsc.toml` (created on first
    /// write command).
    Default(PathBuf),
}

impl ConfigSource {
    /// Resolved path, regardless of how it was selected.
    pub fn path(&self) -> &Path {
        match self {
            Self::Flag(p) | Self::EnvVar(p) | Self::Discovered(p) | Self::Default(p) => p,
        }
    }

    /// Short human label for the source, e.g. `via --config flag`.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Flag(_) => "via --config flag",
            Self::EnvVar(_) => "via $DSC_CONFIG",
            Self::Discovered(_) => "from search hierarchy",
            Self::Default(_) => "default (no config found)",
        }
    }
}

/// Resolve which config file to use, honouring the documented precedence:
///
/// 1. `--config <path>` / `-c` flag
/// 2. `$DSC_CONFIG` env var
/// 3. `./dsc.toml`
/// 4. `$DSC_CONFIG_HOME/dsc.toml` (default: `$XDG_CONFIG_HOME/dsc` -> `~/.config/dsc`)
/// 5. `$XDG_CONFIG_DIRS` entries (Unix only)
/// 6. `/etc/dsc/dsc.toml`, `/etc/dsc.toml`, `/usr/local/etc/dsc.toml` (Unix only)
///
/// Explicit selectors (1, 2) error if the named file does not exist; the
/// discovered hierarchy (3-6) silently skips missing entries. If nothing
/// matches, falls back to `./dsc.toml`.
pub fn resolve_config_source(flag: Option<PathBuf>) -> Result<ConfigSource> {
    resolve_config_source_with_env(flag, |k| std::env::var_os(k))
}

fn resolve_config_source_with_env<F>(flag: Option<PathBuf>, env: F) -> Result<ConfigSource>
where
    F: Fn(&str) -> Option<OsString> + Copy,
{
    if let Some(path) = flag {
        let path = expand_tilde_path(path).map_err(anyhow::Error::msg)?;
        if !path.exists() {
            return Err(anyhow!(
                "config file not found: {} (specified via --config)",
                path.display()
            ));
        }
        return Ok(ConfigSource::Flag(path));
    }

    if let Some(raw) = env(ENV_CONFIG) {
        let path = expand_tilde_path(PathBuf::from(raw)).map_err(anyhow::Error::msg)?;
        if !path.exists() {
            return Err(anyhow!(
                "config file not found: {} (specified via ${})",
                path.display(),
                ENV_CONFIG
            ));
        }
        return Ok(ConfigSource::EnvVar(path));
    }

    let candidates = config_search_paths_with_env(env);
    if let Some(found) = candidates.into_iter().find(|c| c.exists()) {
        return Ok(ConfigSource::Discovered(found));
    }

    Ok(ConfigSource::Default(PathBuf::from("dsc.toml")))
}

/// Returns the ordered list of candidate paths that `dsc` searches for a
/// config file when neither `--config` nor `$DSC_CONFIG` is set.
///
/// Order (first match wins):
/// 1. `./dsc.toml`
/// 2. `$DSC_CONFIG_HOME/dsc.toml` (default: `$XDG_CONFIG_HOME/dsc` -> `~/.config/dsc`)
/// 3. `$XDG_CONFIG_DIRS` entries as `<dir>/dsc/dsc.toml` (Unix only)
/// 4. `/etc/dsc/dsc.toml` (Unix only)
/// 5. `/etc/dsc.toml` (Unix only)
/// 6. `/usr/local/etc/dsc.toml` (Unix only)
pub fn config_search_paths() -> Vec<PathBuf> {
    config_search_paths_with_env(|k| std::env::var_os(k))
}

fn config_search_paths_with_env<F>(env: F) -> Vec<PathBuf>
where
    F: Fn(&str) -> Option<OsString>,
{
    let mut candidates = vec![PathBuf::from("dsc.toml")];

    // $DSC_CONFIG_HOME -> $XDG_CONFIG_HOME/dsc -> $HOME/.config/dsc
    let config_home: Option<PathBuf> = env(ENV_CONFIG_HOME)
        .map(PathBuf::from)
        .map(expand_tilde_or_original)
        .or_else(|| {
            env("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .map(expand_tilde_or_original)
                .map(|x| x.join("dsc"))
        })
        .or_else(|| env("HOME").map(|h| PathBuf::from(h).join(".config").join("dsc")));
    if let Some(dir) = config_home {
        candidates.push(dir.join("dsc.toml"));
    }

    #[cfg(unix)]
    {
        if let Some(xdg_config_dirs) = env("XDG_CONFIG_DIRS") {
            for dir in std::env::split_paths(&xdg_config_dirs) {
                candidates.push(expand_tilde_or_original(dir).join("dsc").join("dsc.toml"));
            }
        } else {
            candidates.push(PathBuf::from("/etc/xdg/dsc/dsc.toml"));
        }
        candidates.push(PathBuf::from("/etc/dsc/dsc.toml"));
        candidates.push(PathBuf::from("/etc/dsc.toml"));
        candidates.push(PathBuf::from("/usr/local/etc/dsc.toml"));
    }

    candidates
}

fn expand_tilde_or_original(path: PathBuf) -> PathBuf {
    expand_tilde_path(path.clone()).unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::ffi::{OsStr, OsString};
    use std::path::PathBuf;

    /// Build an env lookup closure over a fixed map. `None` for missing.
    fn env_from<'a>(
        pairs: &'a HashMap<&'static str, OsString>,
    ) -> impl Fn(&str) -> Option<OsString> + Copy + 'a {
        move |k: &str| pairs.get(k).cloned()
    }

    fn osstr<S: AsRef<OsStr>>(s: S) -> OsString {
        s.as_ref().to_os_string()
    }

    #[test]
    fn discourse_config_debug_redacts_api_key() {
        let discourse = DiscourseConfig {
            name: "private".to_string(),
            baseurl: "https://private.example".to_string(),
            apikey: Some("never-print-this-secret".to_string()),
            ..DiscourseConfig::default()
        };

        let debug = format!("{discourse:?}");
        assert!(debug.contains("#####REDACTED#####"));
        assert!(!debug.contains("never-print-this-secret"));
    }

    #[test]
    fn update_colour_parses_from_toml() {
        let config: Config = toml::from_str(
            r##"
            [[discourse]]
            name = "myforum"
            baseurl = "https://forum.example.com"
            update_colour = "#3f8f77"
            "##,
        )
        .unwrap();
        assert_eq!(
            config.discourse[0].update_colour.as_deref(),
            Some("#3f8f77")
        );
    }

    #[test]
    fn update_colour_empty_string_is_none() {
        let config: Config = toml::from_str(
            r#"
            [[discourse]]
            name = "myforum"
            baseurl = "https://forum.example.com"
            update_colour = ""
            "#,
        )
        .unwrap();
        assert_eq!(config.discourse[0].update_colour, None);
    }

    #[test]
    fn warn_on_invalid_update_colour_does_not_panic_on_malformed_or_valid_values() {
        let config = Config {
            discourse: vec![
                DiscourseConfig {
                    name: "bad".to_string(),
                    baseurl: "https://bad.example".to_string(),
                    update_colour: Some("not-a-colour".to_string()),
                    ..DiscourseConfig::default()
                },
                DiscourseConfig {
                    name: "good".to_string(),
                    baseurl: "https://good.example".to_string(),
                    update_colour: Some("#3f8f77".to_string()),
                    ..DiscourseConfig::default()
                },
            ],
            ..Config::default()
        };
        warn_on_invalid_update_colour(&config);
    }

    #[test]
    fn flag_wins_over_env_and_discovery() {
        let dir = tempfile::tempdir().expect("tempdir");
        let flag_file = dir.path().join("flag.toml");
        let env_file = dir.path().join("env.toml");
        std::fs::write(&flag_file, "").unwrap();
        std::fs::write(&env_file, "").unwrap();

        let mut env = HashMap::new();
        env.insert(ENV_CONFIG, osstr(&env_file));
        let source =
            resolve_config_source_with_env(Some(flag_file.clone()), env_from(&env)).unwrap();
        assert!(matches!(source, ConfigSource::Flag(_)));
        assert_eq!(source.path(), flag_file);
    }

    #[test]
    fn missing_flag_path_errors() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("nope.toml");
        let env: HashMap<&'static str, OsString> = HashMap::new();
        let err = resolve_config_source_with_env(Some(missing), env_from(&env)).unwrap_err();
        assert!(err.to_string().contains("--config"));
    }

    #[test]
    fn dsc_config_env_wins_over_discovery() {
        let dir = tempfile::tempdir().expect("tempdir");
        let env_file = dir.path().join("env.toml");
        std::fs::write(&env_file, "").unwrap();
        let home_dir = dir.path().join("home");
        let dsc_dir = home_dir.join(".config").join("dsc");
        std::fs::create_dir_all(&dsc_dir).unwrap();
        std::fs::write(dsc_dir.join("dsc.toml"), "").unwrap();

        let mut env = HashMap::new();
        env.insert(ENV_CONFIG, osstr(&env_file));
        env.insert("HOME", osstr(&home_dir));
        let source = resolve_config_source_with_env(None, env_from(&env)).unwrap();
        assert!(matches!(source, ConfigSource::EnvVar(_)));
        assert_eq!(source.path(), env_file);
    }

    #[test]
    fn missing_dsc_config_env_path_errors() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("missing.toml");
        let mut env = HashMap::new();
        env.insert(ENV_CONFIG, osstr(&missing));
        let err = resolve_config_source_with_env(None, env_from(&env)).unwrap_err();
        assert!(err.to_string().contains("$DSC_CONFIG"));
    }

    #[test]
    fn dsc_config_home_redirects_step_4() {
        let dir = tempfile::tempdir().expect("tempdir");
        let custom_home = dir.path().join("custom");
        std::fs::create_dir_all(&custom_home).unwrap();
        std::fs::write(custom_home.join("dsc.toml"), "").unwrap();

        let mut env = HashMap::new();
        env.insert(ENV_CONFIG_HOME, osstr(&custom_home));
        let candidates = config_search_paths_with_env(env_from(&env));

        // Step 1: ./dsc.toml; step 2: $DSC_CONFIG_HOME/dsc.toml
        assert_eq!(candidates[0], PathBuf::from("dsc.toml"));
        assert_eq!(candidates[1], custom_home.join("dsc.toml"));
    }

    #[test]
    fn unset_config_home_reproduces_home_config_dsc() {
        // With nothing set except HOME, step 2 must resolve to
        // $HOME/.config/dsc/dsc.toml (today's behaviour).
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().to_path_buf();
        let mut env = HashMap::new();
        env.insert("HOME", osstr(&home));
        let candidates = config_search_paths_with_env(env_from(&env));
        assert_eq!(candidates[0], PathBuf::from("dsc.toml"));
        assert_eq!(
            candidates[1],
            home.join(".config").join("dsc").join("dsc.toml")
        );
    }

    #[test]
    fn xdg_config_home_default_used_when_dsc_config_home_unset() {
        // $XDG_CONFIG_HOME set, $DSC_CONFIG_HOME unset -> step 2 is
        // $XDG_CONFIG_HOME/dsc/dsc.toml.
        let dir = tempfile::tempdir().expect("tempdir");
        let xdg = dir.path().join("xdg");
        let mut env = HashMap::new();
        env.insert("XDG_CONFIG_HOME", osstr(&xdg));
        let candidates = config_search_paths_with_env(env_from(&env));
        assert_eq!(candidates[1], xdg.join("dsc").join("dsc.toml"));
    }

    #[test]
    fn dsc_config_home_overrides_xdg_config_home() {
        let dir = tempfile::tempdir().expect("tempdir");
        let xdg = dir.path().join("xdg");
        let dsc_home = dir.path().join("custom_dsc_home");
        let mut env = HashMap::new();
        env.insert("XDG_CONFIG_HOME", osstr(&xdg));
        env.insert(ENV_CONFIG_HOME, osstr(&dsc_home));
        let candidates = config_search_paths_with_env(env_from(&env));
        assert_eq!(candidates[1], dsc_home.join("dsc.toml"));
    }

    #[test]
    fn unset_everything_resolution_matches_legacy_order() {
        // Regression guard: with no env set, search order must be
        // exactly:
        //   1. ./dsc.toml
        //   (no step 2: no HOME -> no config-home candidate)
        //   3+. Unix system paths
        let env: HashMap<&'static str, OsString> = HashMap::new();
        let candidates = config_search_paths_with_env(env_from(&env));
        assert_eq!(candidates[0], PathBuf::from("dsc.toml"));
        #[cfg(unix)]
        {
            assert!(candidates.contains(&PathBuf::from("/etc/xdg/dsc/dsc.toml")));
            assert!(candidates.contains(&PathBuf::from("/etc/dsc/dsc.toml")));
            assert!(candidates.contains(&PathBuf::from("/etc/dsc.toml")));
            assert!(candidates.contains(&PathBuf::from("/usr/local/etc/dsc.toml")));
        }
    }

    #[test]
    fn no_config_anywhere_returns_default() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Point HOME at an empty dir so step 2 misses too. CWD-relative
        // `dsc.toml` may or may not exist depending on test runner pwd,
        // so we just assert the Default variant is reachable when nothing
        // is set.
        let mut env = HashMap::new();
        env.insert("HOME", osstr(dir.path()));
        // Only assert the source type when ./dsc.toml truly does not exist.
        if !PathBuf::from("dsc.toml").exists() {
            let source = resolve_config_source_with_env(None, env_from(&env)).unwrap();
            assert!(matches!(source, ConfigSource::Default(_)));
        }
    }
}

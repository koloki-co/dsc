// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use anyhow::{Context, Result, anyhow};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static ATOMIC_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Parse a CLI path, expanding a bare `~` or leading `~/` against the user's
/// home directory. Shells do not expand tildes in quoted or `--flag=~/path`
/// arguments, so every CLI path uses this parser.
pub fn tilde_pathbuf(value: &str) -> std::result::Result<PathBuf, String> {
    expand_tilde_with_home(value, home_dir().as_deref())
}

/// Expand a leading tilde while retaining a string-valued CLI argument. Used
/// for hybrid arguments that accept either a local path or a remote resource.
pub fn tilde_path_string(value: &str) -> std::result::Result<String, String> {
    if value != "~" && !value.starts_with("~/") {
        return Ok(value.to_string());
    }
    tilde_pathbuf(value)?
        .into_os_string()
        .into_string()
        .map_err(|_| "expanded path is not valid UTF-8".to_string())
}

/// Expand a leading tilde in a path already obtained from another source, such
/// as an environment variable.
pub fn expand_tilde_path(path: PathBuf) -> std::result::Result<PathBuf, String> {
    let Some(value) = path.to_str() else {
        return Ok(path);
    };
    tilde_pathbuf(value)
}

fn expand_tilde_with_home(
    value: &str,
    home: Option<&Path>,
) -> std::result::Result<PathBuf, String> {
    let suffix = if value == "~" {
        Some("")
    } else {
        value.strip_prefix("~/").or_else(|| {
            if cfg!(windows) {
                value.strip_prefix("~\\")
            } else {
                None
            }
        })
    };
    let Some(suffix) = suffix else {
        return Ok(PathBuf::from(value));
    };
    let home = home.ok_or_else(|| "cannot expand '~': HOME is not set".to_string())?;
    Ok(if suffix.is_empty() {
        home.to_path_buf()
    } else {
        home.join(suffix)
    })
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Trim trailing slashes from a base URL.
pub fn normalize_baseurl(baseurl: &str) -> String {
    baseurl.trim_end_matches('/').to_string()
}

/// Create a URL-safe slug from arbitrary input.
///
/// Wraps the [`slug`] crate, which transliterates Unicode (so `"Café"`
/// becomes `"cafe"`, Cyrillic and CJK get sensible romanisations) and
/// emits the standard kebab-case shape used across most slug-generating
/// tooling. Returns `"untitled"` when the slug would otherwise be empty
/// (the `slug` crate itself returns an empty string for input that has
/// no transliterable characters).
pub fn slugify(input: &str) -> String {
    let s = slug::slugify(input);
    if s.is_empty() {
        "untitled".to_string()
    } else {
        s
    }
}

/// Ensure a directory exists.
pub fn ensure_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path).with_context(|| format!("creating {}", path.display()))?;
    Ok(())
}

/// Create a directory and restrict it to its owner on Unix.
pub fn ensure_private_dir(path: &Path) -> Result<()> {
    ensure_dir(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .with_context(|| format!("setting private permissions on {}", path.display()))?;
    }
    Ok(())
}

/// A same-directory temporary file committed to its destination by rename.
pub struct AtomicOutput {
    file: Option<File>,
    temporary_path: PathBuf,
    destination: PathBuf,
    overwrite: bool,
    committed: bool,
}

impl AtomicOutput {
    pub fn file_mut(&mut self) -> &mut File {
        self.file.as_mut().expect("atomic output already committed")
    }

    pub fn commit(mut self) -> Result<()> {
        if let Some(mut file) = self.file.take() {
            file.flush()
                .with_context(|| format!("flushing {}", self.temporary_path.display()))?;
            file.sync_all()
                .with_context(|| format!("syncing {}", self.temporary_path.display()))?;
        }
        reject_symlink(&self.destination)?;
        if self.destination.exists() && !self.overwrite {
            return Err(anyhow!(
                "refusing to overwrite {}; pass --force to replace it",
                self.destination.display()
            ));
        }
        #[cfg(windows)]
        if self.destination.exists() {
            fs::remove_file(&self.destination)
                .with_context(|| format!("replacing {}", self.destination.display()))?;
        }
        fs::rename(&self.temporary_path, &self.destination).with_context(|| {
            format!(
                "committing {} to {}",
                self.temporary_path.display(),
                self.destination.display()
            )
        })?;
        self.committed = true;
        Ok(())
    }
}

impl Drop for AtomicOutput {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_file(&self.temporary_path);
        }
    }
}

/// Create an atomic output file in the destination directory.
pub fn create_atomic_output(path: &Path, overwrite: bool, private: bool) -> Result<AtomicOutput> {
    ensure_output_available(path, overwrite)?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    ensure_dir(parent)?;
    let file_name = path
        .file_name()
        .ok_or_else(|| anyhow!("output path has no filename: {}", path.display()))?
        .to_string_lossy();

    for _ in 0..100 {
        let sequence = ATOMIC_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temporary_path = parent.join(format!(
            ".{}.dsc-tmp-{}-{}",
            file_name,
            std::process::id(),
            sequence
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(if private { 0o600 } else { 0o644 });
        }
        match options.open(&temporary_path) {
            Ok(file) => {
                return Ok(AtomicOutput {
                    file: Some(file),
                    temporary_path,
                    destination: path.to_path_buf(),
                    overwrite,
                    committed: false,
                });
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("creating temporary file for {}", path.display()));
            }
        }
    }
    Err(anyhow!(
        "could not allocate temporary file for {}",
        path.display()
    ))
}

/// Validate that an output path is safe and available before a multi-file pull.
pub fn ensure_output_available(path: &Path, overwrite: bool) -> Result<()> {
    reject_symlink(path)?;
    if path.exists() && !overwrite {
        return Err(anyhow!(
            "refusing to overwrite {}; pass --force to replace it",
            path.display()
        ));
    }
    Ok(())
}

fn reject_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(anyhow!(
            "refusing to write through symbolic link {}",
            path.display()
        )),
        Ok(_) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("inspecting {}", path.display())),
    }
}

/// Atomically write a regular output file.
pub fn atomic_write(path: &Path, content: impl AsRef<[u8]>, overwrite: bool) -> Result<()> {
    let mut output = create_atomic_output(path, overwrite, false)?;
    output
        .file_mut()
        .write_all(content.as_ref())
        .with_context(|| format!("writing {}", path.display()))?;
    output.commit()
}

/// Atomically write a file restricted to its owner on Unix.
pub fn atomic_write_private(path: &Path, content: impl AsRef<[u8]>, overwrite: bool) -> Result<()> {
    let mut output = create_atomic_output(path, overwrite, true)?;
    output
        .file_mut()
        .write_all(content.as_ref())
        .with_context(|| format!("writing {}", path.display()))?;
    output.commit()
}

/// Resolve a topic path from a user-provided path and a topic title.
pub fn resolve_topic_path(
    provided: Option<&Path>,
    title: &str,
    default_dir: &Path,
) -> Result<PathBuf> {
    let filename = format!("{}.md", slugify(title));
    match provided {
        Some(path) if path.exists() && path.is_dir() => Ok(path.join(filename)),
        Some(path) if path.extension().is_some() => Ok(path.to_path_buf()),
        Some(path) => Ok(path.join(filename)),
        None => Ok(default_dir.join(filename)),
    }
}

/// Read a Markdown file.
pub fn read_markdown(path: &Path) -> Result<String> {
    let raw = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(raw)
}

/// Write a Markdown file, creating parent directories if needed.
pub fn write_markdown(path: &Path, content: &str) -> Result<()> {
    atomic_write(path, content, true)
}

/// Write a pulled Markdown file, refusing replacement unless requested.
pub fn write_markdown_output(path: &Path, content: &str, overwrite: bool) -> Result<()> {
    atomic_write(path, content, overwrite)
}

/// Write sensitive Markdown with owner-only permissions.
pub fn write_markdown_private(path: &Path, content: &str, overwrite: bool) -> Result<()> {
    atomic_write_private(path, content, overwrite)
}

/// Quote a YAML scalar if it contains characters that would confuse the
/// parser. Keeps simple values unquoted. Shared by every command that
/// writes YAML front matter (`topic pull --full`, `category pull`).
pub fn yaml_scalar(value: &str) -> String {
    let needs_quoting = value.is_empty()
        || value.contains(':')
        || value.contains('#')
        || value.contains('\n')
        || value.starts_with([
            '-', '?', '!', '&', '*', '|', '>', '@', '`', '%', '\'', '"', '[',
        ])
        || value.starts_with("  ");
    if needs_quoting {
        let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
        format!("\"{}\"", escaped)
    } else {
        value.to_string()
    }
}

/// Split a Markdown document into its leading YAML front matter (if any) and
/// the body that follows.
///
/// Front matter is recognised only when the file's very first line is exactly
/// `---` (an optional leading BOM is tolerated), terminated by a later line
/// that is exactly `---`. The fenced block is parsed shallowly into a flat
/// `key → value` map (one `key: value` pair per line; lines without a colon
/// are ignored) — `dsc` front matter is intentionally shallow (`title`,
/// `topic_id`, `url`, `pulled_at`), so a full YAML parse is unnecessary and a
/// flat scan keeps the body intact.
///
/// Returns `(map, body)`. When there is no recognisable front matter the map
/// is empty and the body is the original content unchanged, so callers can
/// treat "no front matter" and "empty front matter" identically. One blank
/// line separating the closing fence from the body is consumed (it mirrors
/// what the `pull` side writes), giving a stable pull → push round-trip.
///
/// Note the inherent ambiguity shared with Jekyll/Hugo: a file with no front
/// matter whose body genuinely opens with a `---` thematic break followed by
/// another `---` will be misread as front matter. This is accepted; real
/// snapshots written by `dsc` always carry proper front matter.
pub fn strip_frontmatter(raw: &str) -> (HashMap<String, String>, String) {
    let mut map = HashMap::new();
    let text = raw.strip_prefix('\u{feff}').unwrap_or(raw);

    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return (map, raw.to_string());
    }

    let mut body_lines: Vec<&str> = Vec::new();
    let mut closed = false;
    for line in &mut lines {
        if line.trim_end() == "---" {
            closed = true;
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            map.insert(key.trim().to_string(), unquote_yaml_scalar(value.trim()));
        }
    }

    if !closed {
        // Opening fence with no matching close: not front matter after all.
        return (HashMap::new(), raw.to_string());
    }

    body_lines.extend(lines);
    // Consume a single conventional blank line between fence and body.
    if body_lines.first() == Some(&"") {
        body_lines.remove(0);
    }
    let mut body = body_lines.join("\n");
    if raw.ends_with('\n') && !body.is_empty() {
        body.push('\n');
    }
    (map, body)
}

/// Inverse of [`yaml_scalar`]'s quoting: if `value` is wrapped in double
/// quotes, strip them and unescape `\"` and `\\`. Bare values pass through
/// unchanged, so a value Discourse never sees as quoted (an integer, a URL)
/// is untouched.
fn unquote_yaml_scalar(value: &str) -> String {
    let bytes = value.as_bytes();
    if bytes.len() < 2 || bytes[0] != b'"' || bytes[bytes.len() - 1] != b'"' {
        return value.to_string();
    }
    let inner = &value[1..value.len() - 1];
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Current time in `YYYY-MM-DDTHH:MM:SSZ` form, derived directly from
/// `SystemTime` to avoid a chrono dependency where one is not otherwise
/// needed. Used for the `pulled_at` front-matter stamp.
pub fn current_utc_iso8601() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Days-from-epoch arithmetic (proleptic Gregorian via the standard
    // 1970-01-01 epoch). Good for any year `dsc` will plausibly run in.
    let days = (secs / 86_400) as i64;
    let secs_of_day = secs % 86_400;
    let hh = secs_of_day / 3600;
    let mm = (secs_of_day % 3600) / 60;
    let ss = secs_of_day % 60;
    let (y, m, d) = civil_from_days(days);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, m, d, hh, mm, ss)
}

/// Convert days-from-1970-01-01 to (year, month, day).
/// Reference: Howard Hinnant, "chrono-Compatible Low-Level Date Algorithms".
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = y + if m <= 2 { 1 } else { 0 };
    (y as i32, m as u32, d as u32)
}

fn color_mode() -> &'static str {
    match std::env::var("DSC_COLOR") {
        Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
            "always" => "always",
            "never" => "never",
            _ => "auto",
        },
        Err(_) => "auto",
    }
}

fn color_allowed_for_stdout() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    match color_mode() {
        "always" => true,
        "never" => false,
        _ => std::io::stdout().is_terminal(),
    }
}

fn discourse_color_code(key: &str) -> u8 {
    const COLORS: [u8; 12] = [31, 32, 33, 34, 35, 36, 91, 92, 93, 94, 95, 96];
    let hash = key.bytes().fold(0usize, |acc, b| {
        acc.wrapping_mul(31).wrapping_add(b as usize)
    });
    COLORS[hash % COLORS.len()]
}

/// Parse a strict `#RRGGBB` hex colour into its RGB components. Any other
/// shape (missing `#`, wrong length, non-hex digits, short `#RGB` forms)
/// returns `None` so callers can fall back rather than error.
pub fn parse_hex_color(value: &str) -> Option<(u8, u8, u8)> {
    let digits = value.strip_prefix('#')?;
    if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let r = u8::from_str_radix(&digits[0..2], 16).ok()?;
    let g = u8::from_str_radix(&digits[2..4], 16).ok()?;
    let b = u8::from_str_radix(&digits[4..6], 16).ok()?;
    Some((r, g, b))
}

/// Render `label` with `update_colour` (a validated `#RRGGBB`) as a 24-bit
/// ANSI truecolour, falling back to the deterministic hash-based colour
/// keyed on `key` when `update_colour` is absent or does not parse.
fn discourse_label_ansi(label: &str, key: &str, update_colour: Option<&str>) -> String {
    if let Some((r, g, b)) = update_colour.and_then(parse_hex_color) {
        return format!("\x1b[1;38;2;{};{};{}m{}\x1b[0m", r, g, b, label);
    }
    let code = discourse_color_code(key);
    format!("\x1b[1;{}m{}\x1b[0m", code, label)
}

/// Colour a per-forum label for terminal output. `update_colour`, when
/// `Some` and a valid `#RRGGBB` value, is used as a truecolour override;
/// otherwise `key` (typically the Discourse's `name`) is hashed into one
/// of twelve ANSI colours. Respects `NO_COLOR`/`DSC_COLOR` and returns the
/// plain label unchanged when colour output is not allowed.
pub fn color_discourse_label(label: &str, key: &str, update_colour: Option<&str>) -> String {
    if !color_allowed_for_stdout() {
        return label.to_string();
    }
    discourse_label_ansi(label, key, update_colour)
}

/// Parse a `--since`-style value. Accepts either a relative duration
/// (`7d`, `24h`, `30m`, `1w`, `90s`) or an ISO-8601 absolute timestamp
/// (`2026-04-01`, `2026-04-01T12:00:00Z`). Returns the resulting cutoff
/// instant (now - duration, or the ISO value itself).
pub fn parse_since_cutoff(input: &str) -> anyhow::Result<chrono::DateTime<chrono::Utc>> {
    use anyhow::anyhow;
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("empty --since value"));
    }

    if let Some(duration) = parse_relative_duration(trimmed) {
        return Ok(chrono::Utc::now() - duration);
    }

    // Try RFC3339 (full timestamp).
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(trimmed) {
        return Ok(dt.with_timezone(&chrono::Utc));
    }
    // Try date-only — treat as midnight UTC.
    if let Ok(d) = chrono::NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") {
        return Ok(chrono::NaiveDateTime::new(
            d,
            chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
        )
        .and_utc());
    }

    Err(anyhow!(
        "unrecognised --since value: {:?} (expected e.g. `7d`, `24h`, `30m`, `1w`, or an ISO-8601 timestamp)",
        input
    ))
}

/// Parse a relative duration like `7d`, `24h`, `1w`, `1m`, `90s`, `1y`.
///
/// Calendar units (`m`, `y`) are imprecise; for windows we use these
/// conventions:
///
/// - `s` — seconds
/// - `min` — minutes (use this rather than `m` to avoid the months-vs-minutes
///   ambiguity)
/// - `h` — hours
/// - `d` — days
/// - `w` — weeks (= 7 days)
/// - `m` — **months** (= 30 days; matches what most users mean by "1m" in
///   analytics windows)
/// - `y` — years (= 365 days)
///
/// For exact calendar math, pass an ISO-8601 timestamp instead.
pub fn parse_relative_duration(input: &str) -> Option<chrono::Duration> {
    let s = input.trim();
    if s.len() < 2 {
        return None;
    }
    // Order matters: `min` must be tried before `m` so we don't read
    // "10min" as "10mi" + "n".
    let multi_char_units = [("min", 60i64)];
    for (suffix, secs_per_unit) in multi_char_units {
        if let Some(digits) = s.strip_suffix(suffix) {
            let n: i64 = digits.parse().ok()?;
            return Some(chrono::Duration::seconds(n * secs_per_unit));
        }
    }
    let (digits, unit) = s.split_at(s.len() - 1);
    let n: i64 = digits.parse().ok()?;
    match unit {
        "s" => Some(chrono::Duration::seconds(n)),
        "h" => Some(chrono::Duration::hours(n)),
        "d" => Some(chrono::Duration::days(n)),
        "w" => Some(chrono::Duration::weeks(n)),
        "m" => Some(chrono::Duration::days(n * 30)),
        "y" => Some(chrono::Duration::days(n * 365)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn atomic_write_requires_explicit_overwrite() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("snapshot.txt");
        atomic_write(&path, "first", false).unwrap();
        assert!(atomic_write(&path, "second", false).is_err());
        atomic_write(&path, "second", true).unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), "second");
    }

    #[test]
    fn parse_hex_color_accepts_strict_rrggbb() {
        assert_eq!(parse_hex_color("#3f8f77"), Some((0x3f, 0x8f, 0x77)));
        assert_eq!(parse_hex_color("#FFFFFF"), Some((255, 255, 255)));
    }

    #[test]
    fn parse_hex_color_rejects_malformed_values() {
        assert_eq!(parse_hex_color("3f8f77"), None); // missing '#'
        assert_eq!(parse_hex_color("#fff"), None); // short form
        assert_eq!(parse_hex_color("#3f8f7"), None); // too short
        assert_eq!(parse_hex_color("#3f8f77a"), None); // too long
        assert_eq!(parse_hex_color("#zzzzzz"), None); // non-hex digits
        assert_eq!(parse_hex_color(""), None);
    }

    #[test]
    fn discourse_label_ansi_uses_truecolour_when_update_colour_valid() {
        let out = discourse_label_ansi("myforum", "myforum", Some("#3f8f77"));
        assert_eq!(out, "\x1b[1;38;2;63;143;119mmyforum\x1b[0m");
    }

    #[test]
    fn discourse_label_ansi_falls_back_to_hash_when_update_colour_absent_or_invalid() {
        let expected = format!("\x1b[1;{}mmyforum\x1b[0m", discourse_color_code("myforum"));
        assert_eq!(discourse_label_ansi("myforum", "myforum", None), expected);
        assert_eq!(
            discourse_label_ansi("myforum", "myforum", Some("not-a-colour")),
            expected
        );
    }

    #[cfg(unix)]
    #[test]
    fn private_atomic_write_uses_owner_only_permissions_and_rejects_symlinks() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let dir = tempdir().unwrap();
        let private_dir = dir.path().join("sar");
        ensure_private_dir(&private_dir).unwrap();
        assert_eq!(
            fs::metadata(&private_dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let path = dir.path().join("private.txt");
        atomic_write_private(&path, "secret", false).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );

        let link = dir.path().join("link.txt");
        symlink(&path, &link).unwrap();
        assert!(atomic_write_private(&link, "replacement", true).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "secret");
    }

    #[test]
    fn tilde_path_expands_bare_and_prefixed_forms() {
        let home = Path::new("/home/example");
        assert_eq!(
            expand_tilde_with_home("~", Some(home)).unwrap(),
            PathBuf::from("/home/example")
        );
        assert_eq!(
            expand_tilde_with_home("~/config/dsc.toml", Some(home)).unwrap(),
            PathBuf::from("/home/example/config/dsc.toml")
        );
    }

    #[test]
    fn tilde_path_preserves_other_paths_and_rejects_missing_home() {
        assert_eq!(
            expand_tilde_with_home("./local.md", None).unwrap(),
            PathBuf::from("./local.md")
        );
        assert_eq!(
            expand_tilde_with_home("~other/file", None).unwrap(),
            PathBuf::from("~other/file")
        );
        assert!(expand_tilde_with_home("~/file", None).is_err());
    }

    #[test]
    fn tilde_path_string_preserves_non_paths() {
        assert_eq!(
            tilde_path_string("https://example.com/theme.git").unwrap(),
            "https://example.com/theme.git"
        );
        assert_eq!(tilde_path_string("forum-name").unwrap(), "forum-name");
    }

    #[test]
    fn slugify_simple_ascii() {
        assert_eq!(slugify("Hello World"), "hello-world");
    }

    #[test]
    fn slugify_collapses_runs_of_non_alnum() {
        assert_eq!(slugify("a   b___c!!!d"), "a-b-c-d");
    }

    #[test]
    fn slugify_trims_leading_and_trailing_dashes() {
        assert_eq!(slugify("   hello   "), "hello");
        assert_eq!(slugify("!!!foo!!!"), "foo");
    }

    #[test]
    fn slugify_empty_input_returns_untitled() {
        assert_eq!(slugify(""), "untitled");
        assert_eq!(slugify("   "), "untitled");
        assert_eq!(slugify("!!!"), "untitled");
    }

    #[test]
    fn slugify_preserves_numbers() {
        assert_eq!(slugify("Topic 42 - intro"), "topic-42-intro");
    }

    #[test]
    fn slugify_lowercases() {
        assert_eq!(slugify("ABCxyz"), "abcxyz");
    }

    #[test]
    fn slugify_transliterates_unicode() {
        // The whole reason for adopting the `slug` crate: pre-existing
        // ASCII behaviour preserved, plus accented Latin, Cyrillic, and
        // CJK now produce meaningful slugs instead of "untitled".
        assert_eq!(slugify("Café Tonight"), "cafe-tonight");
        assert_eq!(slugify("Привет мир"), "privet-mir");
        assert_eq!(slugify("日本語"), "ri-ben-yu");
    }

    #[test]
    fn slugify_trims_both_ends_of_dashes() {
        // Regression guard: catches a latent bug from a contributor PR
        // that only trimmed trailing dashes. The `slug` crate handles
        // both ends correctly.
        assert_eq!(slugify("-foo-"), "foo");
        assert_eq!(slugify("---foo---bar---"), "foo-bar");
    }

    #[test]
    fn normalize_baseurl_strips_trailing_slashes() {
        assert_eq!(
            normalize_baseurl("https://example.com/"),
            "https://example.com"
        );
        assert_eq!(
            normalize_baseurl("https://example.com///"),
            "https://example.com"
        );
        assert_eq!(
            normalize_baseurl("https://example.com"),
            "https://example.com"
        );
    }

    #[test]
    fn normalize_baseurl_preserves_no_trailing() {
        assert_eq!(normalize_baseurl(""), "");
    }

    #[test]
    fn resolve_topic_path_uses_title_when_no_path_given() {
        let default_dir = Path::new("/tmp/dsc-test");
        let out = resolve_topic_path(None, "Hello World", default_dir).unwrap();
        assert_eq!(out, default_dir.join("hello-world.md"));
    }

    #[test]
    fn resolve_topic_path_uses_given_path_with_extension() {
        let default_dir = Path::new("/tmp/dsc-test");
        let explicit = Path::new("/tmp/custom.md");
        let out = resolve_topic_path(Some(explicit), "Ignored", default_dir).unwrap();
        assert_eq!(out, explicit);
    }

    #[test]
    fn parse_relative_duration_common_units() {
        assert_eq!(
            parse_relative_duration("7d"),
            Some(chrono::Duration::days(7))
        );
        assert_eq!(
            parse_relative_duration("24h"),
            Some(chrono::Duration::hours(24))
        );
        assert_eq!(
            parse_relative_duration("30min"),
            Some(chrono::Duration::minutes(30))
        );
        assert_eq!(
            parse_relative_duration("1w"),
            Some(chrono::Duration::weeks(1))
        );
        assert_eq!(
            parse_relative_duration("90s"),
            Some(chrono::Duration::seconds(90))
        );
    }

    #[test]
    fn parse_relative_duration_rejects_nonsense() {
        assert!(parse_relative_duration("").is_none());
        assert!(parse_relative_duration("d").is_none());
        assert!(parse_relative_duration("7x").is_none());
        assert!(parse_relative_duration("abc").is_none());
        assert!(parse_relative_duration("3M").is_none()); // case-sensitive
    }

    #[test]
    fn parse_relative_duration_treats_m_as_months() {
        // `m` = months (= 30 days). Users naturally write `1m` for "one
        // month" in analytics windows; we match that. Use `min` for the
        // rare minutes case.
        assert_eq!(
            parse_relative_duration("1m"),
            Some(chrono::Duration::days(30))
        );
        assert_eq!(
            parse_relative_duration("3m"),
            Some(chrono::Duration::days(90))
        );
    }

    #[test]
    fn parse_relative_duration_minutes_via_min_suffix() {
        assert_eq!(
            parse_relative_duration("5min"),
            Some(chrono::Duration::minutes(5))
        );
        assert_eq!(
            parse_relative_duration("90min"),
            Some(chrono::Duration::minutes(90))
        );
    }

    #[test]
    fn parse_relative_duration_accepts_years_as_365d() {
        assert_eq!(
            parse_relative_duration("1y"),
            Some(chrono::Duration::days(365))
        );
        assert_eq!(
            parse_relative_duration("2y"),
            Some(chrono::Duration::days(730))
        );
    }

    #[test]
    fn parse_since_cutoff_iso_date() {
        let cutoff = parse_since_cutoff("2026-01-01").unwrap();
        assert_eq!(cutoff.to_rfc3339(), "2026-01-01T00:00:00+00:00");
    }

    #[test]
    fn parse_since_cutoff_iso_timestamp() {
        let cutoff = parse_since_cutoff("2026-04-15T12:30:00Z").unwrap();
        assert_eq!(cutoff.to_rfc3339(), "2026-04-15T12:30:00+00:00");
    }

    #[test]
    fn parse_since_cutoff_relative_is_in_the_past() {
        let now = chrono::Utc::now();
        let cutoff = parse_since_cutoff("7d").unwrap();
        let diff = now - cutoff;
        // Should be very close to 7 days (within a second).
        assert!(
            (diff - chrono::Duration::days(7)).num_seconds().abs() < 2,
            "expected ~7 day delta, got {}",
            diff
        );
    }

    #[test]
    fn parse_since_cutoff_rejects_garbage() {
        assert!(parse_since_cutoff("not a date").is_err());
        assert!(parse_since_cutoff("").is_err());
    }

    #[test]
    fn yaml_scalar_leaves_simple_values_bare() {
        assert_eq!(
            yaml_scalar("Dependency management"),
            "Dependency management"
        );
        assert_eq!(yaml_scalar("Topic 42"), "Topic 42");
    }

    #[test]
    fn yaml_scalar_quotes_when_needed() {
        assert_eq!(yaml_scalar("a: b"), "\"a: b\"");
        assert_eq!(yaml_scalar("# hash"), "\"# hash\"");
        assert_eq!(yaml_scalar("- leading dash"), "\"- leading dash\"");
        // Interior quotes alone do not trigger quoting; a quote that coincides
        // with another trigger (here the colon) is escaped inside the wrap.
        assert_eq!(yaml_scalar("she said \"hi\""), "she said \"hi\"");
        assert_eq!(yaml_scalar("a: \"b\""), "\"a: \\\"b\\\"\"");
    }

    #[test]
    fn strip_frontmatter_parses_block_and_body() {
        let raw = "---\ntitle: Dependency management\ntopic_id: 412\nurl: https://forum.rcpch.tech/t/dependency-management/412\npulled_at: 2026-06-22T09:19:00Z\n---\n\nBody line one.\nBody line two.\n";
        let (front, body) = strip_frontmatter(raw);
        assert_eq!(front.get("topic_id").map(String::as_str), Some("412"));
        assert_eq!(
            front.get("title").map(String::as_str),
            Some("Dependency management")
        );
        assert_eq!(
            front.get("url").map(String::as_str),
            Some("https://forum.rcpch.tech/t/dependency-management/412")
        );
        assert_eq!(body, "Body line one.\nBody line two.\n");
    }

    #[test]
    fn strip_frontmatter_absent_returns_empty_map_and_full_body() {
        let raw = "# Heading\n\nNo front matter here.\n";
        let (front, body) = strip_frontmatter(raw);
        assert!(front.is_empty());
        assert_eq!(body, raw);
    }

    #[test]
    fn strip_frontmatter_unclosed_fence_is_not_front_matter() {
        // Opening `---` but never closed: treat the whole thing as body.
        let raw = "---\ntitle: oops\nstill body, no closing fence\n";
        let (front, body) = strip_frontmatter(raw);
        assert!(front.is_empty());
        assert_eq!(body, raw);
    }

    #[test]
    fn strip_frontmatter_preserves_horizontal_rules_in_body() {
        // A `---` inside the body (after the real close) must survive intact.
        let raw = "---\ntopic_id: 7\n---\n\nIntro.\n\n---\n\nAfter the rule.\n";
        let (front, body) = strip_frontmatter(raw);
        assert_eq!(front.get("topic_id").map(String::as_str), Some("7"));
        assert_eq!(body, "Intro.\n\n---\n\nAfter the rule.\n");
    }

    #[test]
    fn strip_frontmatter_unquotes_yaml_scalar_values() {
        // yaml_scalar quotes a title containing a colon; strip must invert it.
        let title = "Intro: getting started";
        let raw = format!(
            "---\ntitle: {}\ntopic_id: 3\n---\n\nbody\n",
            yaml_scalar(title)
        );
        let (front, body) = strip_frontmatter(&raw);
        assert_eq!(front.get("title").map(String::as_str), Some(title));
        assert_eq!(front.get("topic_id").map(String::as_str), Some("3"));
        assert_eq!(body, "body\n");
    }

    #[test]
    fn strip_frontmatter_leaves_url_with_colons_intact() {
        // URLs are written bare (not via yaml_scalar) and only the first colon
        // separates key from value, so the scheme colon must survive.
        let raw = "---\nurl: https://forum.rcpch.tech/t/x/9\n---\n\nbody\n";
        let (front, _) = strip_frontmatter(raw);
        assert_eq!(
            front.get("url").map(String::as_str),
            Some("https://forum.rcpch.tech/t/x/9")
        );
    }

    #[test]
    fn strip_frontmatter_tolerates_leading_bom() {
        let raw = "\u{feff}---\ntopic_id: 99\n---\n\nbody\n";
        let (front, body) = strip_frontmatter(raw);
        assert_eq!(front.get("topic_id").map(String::as_str), Some("99"));
        assert_eq!(body, "body\n");
    }

    #[test]
    fn current_utc_iso8601_has_expected_shape() {
        let s = current_utc_iso8601();
        assert_eq!(s.len(), 20, "got {s:?}");
        assert!(s.ends_with('Z'));
        assert_eq!(&s[4..5], "-");
        assert_eq!(&s[10..11], "T");
    }

    #[test]
    fn civil_from_days_matches_known_dates() {
        // 1970-01-01 is day 0.
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // 2026-06-10 = 20614 days from epoch (well-known via cal / date).
        assert_eq!(civil_from_days(20614), (2026, 6, 10));
        // Leap-day check: 2024-02-29.
        assert_eq!(civil_from_days(19782), (2024, 2, 29));
    }
}

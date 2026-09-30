// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! Tripwire for a recurring bug class: Discourse's admin API returns
//! **signed** numeric IDs on the wire, and reserves negative values for
//! built-in/system entities — `system` (-1) and `discobot` (-2) among
//! users, Foundation (-1) and Horizon (-2) among themes, and several
//! built-in colour schemes/palettes. Extracting such a field with
//! `serde_json::Value::as_u64()` (or deserializing straight into a `u64`
//! field) does not error loudly on a negative value: `as_u64()` just
//! returns `None`, which every call site so far has chained into
//! `.unwrap_or_default()` (silently becomes `0`) or `.filter_map()`
//! (silently drops the row) — a wrong answer with no error message,
//! discovered three separate times in the field (`system`/`discobot` user
//! rows, then themes) before anyone thought to test for it. See
//! `spec/cli-design.md`'s "Discourse ID types" section for the rule.
//!
//! This test enumerates every `.as_u64()` call site in `src/` and fails if
//! it finds one that isn't in the allowlist below. It is not clever about
//! *why* a site is safe — every entry is a manual, reviewed judgement call,
//! same as the dry-run mutation registry in `dry-run-mutation-test.rs`. The
//! point is to force that judgement call to happen at review time for any
//! *new* call site, rather than silently inheriting the same failure mode
//! for the next Discourse resource type that turns out to have negative
//! IDs.
//!
//! Adding a new `.as_u64()` call site to `src/`? Before adding it to the
//! allowlist below, actually check whether Discourse can return a negative
//! ID for that field (built-in/system rows, defaults, fresh installs) — if
//! it can, use `.as_i64()` and a signed field/parameter type instead, the
//! same fix already applied to users, themes, and colour schemes.

use std::fs;
use std::path::Path;

/// `(file relative to src/, trimmed line content)`. Every line in `src/`
/// containing the literal substring `.as_u64()` must match one of these
/// exactly (comments included — this is a dumb text scan, not a parser).
const ALLOWED_AS_U64_SITES: &[(&str, &str)] = &[
    // Retry-after wait time from a rate-limit response header — a
    // duration, not an entity ID; Discourse never sends a negative wait.
    ("api/rate_limit.rs", ".and_then(|w| w.as_u64())"),
    // Tag group creation response: a *freshly created* tag group always
    // gets a new, positive ID from Discourse — there is no built-in
    // system tag group a create response could ever echo back.
    ("api/tags.rs", ".and_then(|v| v.as_u64())"),
    // Group creation response, three fallback lookup paths
    // (`group`/`basic_group`/top-level `id`): same reasoning as tag
    // groups — a *created* group is never one of Discourse's built-in
    // automatic groups (those are pre-seeded, never returned from a
    // create call).
    ("api/groups.rs", ".and_then(|id| id.as_u64())"),
    (
        "api/groups.rs",
        ".or_else(|| value.get(\"id\").and_then(|id| id.as_u64()))",
    ),
    // Theme creation response: same reasoning — a *created* theme is
    // never one of Discourse's built-in system themes (Foundation/
    // Horizon are pre-seeded, never returned from a create call). The
    // built-in-theme bug this test guards against was in *reading*
    // existing themes (`fetch_theme`/`list_themes`), not creating one.
    ("api/themes.rs", ".and_then(|v| v.as_u64())"),
    // Category permission level: a small fixed enum (1 = full,
    // 2 = create_post, 3 = readonly), not an entity ID; Discourse never
    // sends a negative permission level.
    ("commands/tag.rs", "if let Some(level) = level.as_u64() {"),
    // Backup file size in bytes — a size, not an entity ID.
    ("commands/backup.rs", ".and_then(|v| v.as_u64())"),
    (
        "commands/backup.rs",
        ".or_else(|| backup.get(\"size_bytes\").and_then(|v| v.as_u64()))",
    ),
    // Component (child theme) IDs attached to a parent. Deliberately
    // `u64`, not part of the theme-ID fix: components are always
    // freshly created by a user, never one of Discourse's built-in
    // themes, so never negative. (The parent theme ID in the same
    // function *is* signed — see `theme_set_child`.)
    (
        "commands/theme.rs",
        ".filter_map(|c| c.get(\"id\").and_then(|v| v.as_u64()))",
    ),
    // Comments in the theme regression tests that mention `.as_u64()` as
    // text while describing the bug it caused. Not code.
    (
        "commands/theme.rs",
        "// theme is. Regression test for a bug where `.as_u64()` silently",
    ),
    (
        "commands/theme.rs",
        "// the default themes on any fresh install) at all: `.as_u64()`",
    ),
    (
        "commands/theme.rs",
        "// had the same `.as_u64().unwrap_or_default()` bug, silently",
    ),
];

#[test]
fn every_as_u64_call_site_is_reviewed_and_allowlisted() {
    let src_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut unexpected = Vec::new();
    let mut files = Vec::new();
    collect_rs_files(&src_dir, &mut files);

    for file in &files {
        let relative = file
            .strip_prefix(&src_dir)
            .expect("file must be under src/")
            .to_string_lossy()
            .replace('\\', "/");
        let content =
            fs::read_to_string(file).unwrap_or_else(|e| panic!("reading {}: {e}", file.display()));
        for (line_number, line) in content.lines().enumerate() {
            if !line.contains(".as_u64()") {
                continue;
            }
            let trimmed = line.trim();
            let allowed = ALLOWED_AS_U64_SITES
                .iter()
                .any(|(allowed_file, allowed_line)| {
                    *allowed_file == relative && *allowed_line == trimmed
                });
            if !allowed {
                unexpected.push(format!("src/{relative}:{}: {trimmed}", line_number + 1));
            }
        }
    }

    assert!(
        unexpected.is_empty(),
        "found `.as_u64()` call site(s) not in ALLOWED_AS_U64_SITES:\n{}\n\n\
         `serde_json::Value::as_u64()` returns `None` for a negative number \
         instead of erroring — Discourse's built-in/system entities (users \
         `system`/`discobot`, themes Foundation/Horizon, several colour \
         schemes) use negative IDs on the wire, and this has silently \
         broken three separate `dsc` features so far. Before allowlisting a \
         new site: check whether Discourse can ever return a negative ID \
         for this field. If it can (or you're not sure), use `.as_i64()` \
         and a signed `i64` field/parameter instead — see \
         `spec/cli-design.md`'s \"Discourse ID types\" section. If it \
         genuinely cannot (a freshly-created resource's own ID, a count, a \
         small fixed enum, etc.), add the exact trimmed line to \
         ALLOWED_AS_U64_SITES in this test with a one-line justification, \
         matching the existing entries.",
        unexpected.join("\n")
    );
}

fn collect_rs_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()));
    for entry in entries {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

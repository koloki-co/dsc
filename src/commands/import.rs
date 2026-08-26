// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::commands::common::{fetch_fullnames, parse_tags};
use crate::config::{Config, DiscourseConfig};
use crate::utils::slugify;
use anyhow::{Context, Result};
use std::fs;
use std::io::{self, Read};
use std::path::Path;

pub fn import_discourses(config: &mut Config, path: Option<&Path>) -> Result<()> {
    let mut raw = String::new();
    if let Some(path) = path {
        if path == Path::new("-") {
            io::stdin().read_to_string(&mut raw)?;
        } else {
            raw =
                fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        }
    } else {
        io::stdin().read_to_string(&mut raw)?;
    }
    import_from_string(config, &raw, path)?;
    Ok(())
}

fn import_from_string(config: &mut Config, raw: &str, path: Option<&Path>) -> Result<()> {
    let is_csv = path.and_then(|p| p.extension().and_then(|s| s.to_str())) == Some("csv")
        || looks_like_csv(raw);
    if is_csv {
        import_csv(config, raw)?;
    } else {
        import_text(config, raw)?;
    }
    Ok(())
}

fn import_text(config: &mut Config, raw: &str) -> Result<()> {
    let urls: Vec<String> = raw
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    let fullnames = fetch_fullnames(&urls);
    for (url, fullname) in urls.iter().zip(fullnames.iter()) {
        let fullname = fullname.clone();
        let name = if let Some(title) = fullname.as_deref() {
            slugify(title)
        } else {
            slugify(url)
        };
        config.discourse.push(DiscourseConfig {
            name,
            baseurl: url.clone(),
            fullname,
            ..DiscourseConfig::default()
        });
    }
    Ok(())
}

fn import_csv(config: &mut Config, raw: &str) -> Result<()> {
    let mut reader = csv::Reader::from_reader(raw.as_bytes());
    let rows: Vec<(String, String, Option<Vec<String>>)> = reader
        .records()
        .filter_map(|result| result.ok())
        .map(|record| {
            let name = record.get(0).unwrap_or("").trim().to_string();
            let url = record.get(1).unwrap_or("").trim().to_string();
            let tags = record.get(2).map(parse_tags).filter(|t| !t.is_empty());
            (name, url, tags)
        })
        .filter(|(_, url, _)| !url.is_empty())
        .collect();

    let urls: Vec<String> = rows.iter().map(|(_, url, _)| url.clone()).collect();
    let fullnames = fetch_fullnames(&urls);

    for ((name, url, tags), fullname) in rows.into_iter().zip(fullnames) {
        let name = if name.is_empty() {
            if let Some(title) = fullname.as_deref() {
                slugify(title)
            } else {
                slugify(&url)
            }
        } else {
            name
        };
        config.discourse.push(DiscourseConfig {
            name,
            baseurl: url,
            fullname,
            tags,
            ..DiscourseConfig::default()
        });
    }
    Ok(())
}

fn looks_like_csv(raw: &str) -> bool {
    let first = raw.lines().find(|line| !line.trim().is_empty());
    let Some(first) = first else { return false };
    let lower = first.to_ascii_lowercase();
    lower.contains("name") && lower.contains("url") && first.contains(',')
}

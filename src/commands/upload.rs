// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::api::DiscourseClient;
use crate::cli::ListFormat;
use crate::commands::common::{ensure_api_credentials, select_discourse};
use crate::config::Config;
use anyhow::{Result, anyhow};
use std::path::Path;

pub fn upload(
    config: &Config,
    discourse_name: &str,
    file_path: &Path,
    upload_type: &str,
    format: ListFormat,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    if !file_path.is_file() {
        return Err(anyhow!("not a file: {}", file_path.display()));
    }

    let info = client.upload_file(file_path, upload_type)?;

    match format {
        ListFormat::Text => {
            // Discourse silently converts large PNGs to JPEG server-side
            // (`png_to_jpg_quality` setting) and loses transparency in the
            // process. Flag it here since the short URL alone hides the
            // extension change; JSON/YAML output already carries
            // `original_filename` for callers who parse it themselves.
            if let Some(hint) = conversion_hint(file_path, &info.original_filename) {
                eprintln!("{hint}");
            }
            // Default text output prints just the short URL — that's what
            // gets pasted into post bodies. Single line, pipe-friendly.
            if let Some(short) = &info.short_url {
                println!("{}", short);
            } else {
                println!("{}", info.url);
            }
        }
        ListFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&info)?);
        }
        ListFormat::Yaml => {
            println!("{}", serde_yaml::to_string(&info)?);
        }
    }

    Ok(())
}

/// A note for when the extension Discourse returned differs from the local
/// file's extension — most commonly a PNG converted to JPEG server-side
/// (`UploadCreator`, `png_to_jpg_quality` site setting), which drops
/// transparency. `None` when there's nothing to flag (no extension on
/// either side, or they match case-insensitively).
fn conversion_hint(local_path: &Path, returned_filename: &str) -> Option<String> {
    let local_ext = extension_lower(local_path.file_name()?.to_str()?)?;
    let returned_ext = extension_lower(returned_filename)?;
    if local_ext == returned_ext {
        return None;
    }
    Some(format!(
        "note: Discourse converted this upload from .{local_ext} to .{returned_ext} \
         (returned as \"{returned_filename}\"); if this was a transparent PNG the \
         transparency is now lost. Use --upload-type custom_emoji to skip conversion, \
         or set png_to_jpg_quality to 100 to disable it site-wide."
    ))
}

fn extension_lower(filename: &str) -> Option<String> {
    filename
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .filter(|ext| !ext.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_a_png_to_jpeg_conversion() {
        let hint = conversion_hint(Path::new("logo.png"), "logo.jpeg").unwrap();
        assert!(hint.contains(".png to .jpeg"));
        assert!(hint.contains("logo.jpeg"));
    }

    #[test]
    fn is_case_insensitive_and_silent_when_extensions_match() {
        assert!(conversion_hint(Path::new("logo.PNG"), "logo.png").is_none());
    }

    #[test]
    fn is_silent_when_no_conversion_happened() {
        assert!(conversion_hint(Path::new("diagram.png"), "diagram.png").is_none());
    }

    #[test]
    fn handles_missing_extensions_without_panicking() {
        assert!(conversion_hint(Path::new("README"), "README").is_none());
        assert!(conversion_hint(Path::new("README"), "README.png").is_none());
        assert!(conversion_hint(Path::new("logo.png"), "README").is_none());
    }
}

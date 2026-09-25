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
            // The short URL alone hides server-side filename changes;
            // JSON/YAML output already carries `original_filename`.
            if let Some(hint) = extension_change_hint(file_path, &info.original_filename) {
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

/// A note when the extension returned by Discourse differs from the local
/// file's extension. `None` when either extension is absent or they match.
fn extension_change_hint(local_path: &Path, returned_filename: &str) -> Option<String> {
    let local_ext = extension_lower(local_path)?;
    let returned_ext = extension_lower(Path::new(returned_filename))?;
    if local_ext == returned_ext {
        return None;
    }

    let change = format!(
        "note: Discourse returned this upload as \"{returned_filename}\" \
         (.{returned_ext} instead of .{local_ext})."
    );
    if local_ext == "png" && matches!(returned_ext.as_str(), "jpg" | "jpeg") {
        return Some(format!(
            "{change} PNG-to-JPEG conversion loses transparency; use --upload-type \
             custom_emoji to skip conversion, or set png_to_jpg_quality to 100 to \
             disable it site-wide."
        ));
    }
    Some(change)
}

fn extension_lower(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .filter(|ext| !ext.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_a_png_to_jpeg_conversion() {
        for returned in ["logo.jpg", "logo.jpeg"] {
            let hint = extension_change_hint(Path::new("logo.png"), returned).unwrap();
            assert!(hint.contains(returned));
            assert!(hint.contains("PNG-to-JPEG conversion loses transparency"));
            assert!(hint.contains("png_to_jpg_quality"));
        }
    }

    #[test]
    fn describes_other_extension_changes_without_claiming_conversion() {
        let hint = extension_change_hint(Path::new("photo.jpg"), "photo.jpeg").unwrap();
        assert_eq!(
            hint,
            "note: Discourse returned this upload as \"photo.jpeg\" \
             (.jpeg instead of .jpg)."
        );
    }

    #[test]
    fn is_case_insensitive_and_silent_when_extensions_match() {
        assert!(extension_change_hint(Path::new("logo.PNG"), "logo.png").is_none());
    }

    #[test]
    fn is_silent_when_no_conversion_happened() {
        assert!(extension_change_hint(Path::new("diagram.png"), "diagram.png").is_none());
    }

    #[test]
    fn handles_missing_extensions_without_panicking() {
        assert!(extension_change_hint(Path::new("README"), "README").is_none());
        assert!(extension_change_hint(Path::new("README"), "README.png").is_none());
        assert!(extension_change_hint(Path::new("logo.png"), "README").is_none());
        assert!(extension_change_hint(Path::new(".png"), ".jpeg").is_none());
    }
}

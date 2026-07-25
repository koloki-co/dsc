// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use anyhow::anyhow;
use reqwest::StatusCode;

pub fn http_error(action: &str, status: StatusCode, text: &str) -> anyhow::Error {
    let trimmed = text.trim();
    let hint = hint_for(status, trimmed);
    match (trimmed.is_empty(), hint) {
        (true, Some(h)) => anyhow!("{action} failed with {} — {}", status, h),
        (true, None) => anyhow!("{action} failed with {} (empty response)", status),
        (false, Some(h)) => anyhow!("{action} failed with {} — {}: {}", status, h, trimmed),
        (false, None) => anyhow!("{action} failed with {}: {}", status, trimmed),
    }
}

/// Pick a hint from the status, sharpened by the response body. Discourse's
/// `invalid_access` / "the API username or key is invalid" can arrive as a 403
/// (on content) or a 404 (admin routes are hidden from non-staff), so detect
/// that signature directly rather than guessing from the status alone.
fn hint_for(status: StatusCode, body: &str) -> Option<&'static str> {
    if looks_like_invalid_credentials(body) {
        return Some(
            "the api_username/key for this forum is invalid or not a staff member — \
             verify this forum's entry in your config (run `dsc config check`)",
        );
    }
    status_hint(status)
}

/// Does the body carry Discourse's "you're not authorised / your key is bad"
/// signature? (Returned on a 403 for content, and behind the 404 that hides
/// `/admin/*` from non-staff.)
fn looks_like_invalid_credentials(body: &str) -> bool {
    let b = body.to_ascii_lowercase();
    b.contains("invalid_access") || b.contains("api username or key is invalid")
}

pub(crate) fn status_hint(status: StatusCode) -> Option<&'static str> {
    match status {
        StatusCode::NOT_FOUND => Some(
            "not found — check the resource ID and that the endpoint exists; for an admin \
             action this can also mean the api_username is not a staff member (Discourse \
             hides /admin routes behind 404)",
        ),
        StatusCode::FORBIDDEN => Some(
            "forbidden — the api_username/key may be invalid, not a staff member, or lack \
             the scope for this action; verify this forum's config (`dsc config check`)",
        ),
        StatusCode::UNAUTHORIZED => {
            Some("unauthorized (check apikey and api_username in your config)")
        }
        StatusCode::TOO_MANY_REQUESTS => Some(
            "rate-limited (raise DISCOURSE_MAX_ADMIN_API_REQS_PER_MINUTE or slow the request rate)",
        ),
        StatusCode::UNPROCESSABLE_ENTITY => Some("validation error (see details below)"),
        StatusCode::INTERNAL_SERVER_ERROR
        | StatusCode::BAD_GATEWAY
        | StatusCode::SERVICE_UNAVAILABLE
        | StatusCode::GATEWAY_TIMEOUT => {
            Some("server error (try again; check the Discourse host is healthy)")
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{http_error, status_hint};
    use reqwest::StatusCode;

    #[test]
    fn hint_maps_unauthorized_to_credentials_message() {
        let h = status_hint(StatusCode::UNAUTHORIZED).unwrap();
        assert!(h.contains("apikey"), "expected apikey hint, got {h:?}");
    }

    #[test]
    fn hint_maps_forbidden_to_credentials_and_scope_message() {
        let h = status_hint(StatusCode::FORBIDDEN).unwrap();
        assert!(
            h.contains("api_username/key") && h.contains("config"),
            "expected a credentials+config hint, got {h:?}"
        );
    }

    #[test]
    fn invalid_access_body_yields_credentials_hint_even_on_404() {
        // Discourse hides /admin behind a 404 for non-staff; a 403 on content
        // carries the same invalid_access signature. Both must point at config.
        let body = r#"{"errors":["You are not permitted to view the requested resource. The API username or key is invalid."],"error_type":"invalid_access"}"#;
        for status in [StatusCode::FORBIDDEN, StatusCode::NOT_FOUND] {
            let s = http_error("topic request", status, body).to_string();
            assert!(
                s.contains("invalid or not a staff member") && s.contains("dsc config check"),
                "expected credentials hint for {status}, got {s:?}"
            );
        }
    }

    #[test]
    fn plain_404_hint_mentions_non_staff_admin_case() {
        let h = status_hint(StatusCode::NOT_FOUND).unwrap();
        assert!(h.contains("staff member"), "got {h:?}");
    }

    #[test]
    fn hint_maps_429_to_rate_limit_message() {
        let h = status_hint(StatusCode::TOO_MANY_REQUESTS).unwrap();
        assert!(
            h.contains("DISCOURSE_MAX_ADMIN_API_REQS_PER_MINUTE"),
            "expected rate-limit hint to mention the env var, got {h:?}"
        );
    }

    #[test]
    fn hint_none_for_success_codes() {
        assert!(status_hint(StatusCode::OK).is_none());
        assert!(status_hint(StatusCode::CREATED).is_none());
    }

    #[test]
    fn hint_maps_5xx_to_server_error_message() {
        assert!(status_hint(StatusCode::INTERNAL_SERVER_ERROR).is_some());
        assert!(status_hint(StatusCode::BAD_GATEWAY).is_some());
        assert!(status_hint(StatusCode::SERVICE_UNAVAILABLE).is_some());
    }

    #[test]
    fn http_error_combines_action_status_and_hint() {
        let err = http_error("create widget", StatusCode::UNAUTHORIZED, "");
        let s = err.to_string();
        assert!(s.contains("create widget"));
        assert!(s.contains("401"));
        assert!(s.contains("apikey"));
    }

    #[test]
    fn http_error_includes_body_when_nonempty() {
        let err = http_error(
            "post stuff",
            StatusCode::UNPROCESSABLE_ENTITY,
            "{\"errors\":[\"title must be at least 15 characters\"]}",
        );
        let s = err.to_string();
        assert!(s.contains("post stuff"));
        assert!(s.contains("422"));
        assert!(s.contains("title must be at least 15 characters"));
    }

    #[test]
    fn http_error_with_unknown_status_has_no_hint_suffix() {
        let err = http_error("do thing", StatusCode::IM_A_TEAPOT, "nope");
        let s = err.to_string();
        assert!(s.contains("418"));
        assert!(s.contains("nope"));
    }
}

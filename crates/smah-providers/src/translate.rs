//! Error translation table: HTTP status / transport condition →
//! [`SmahError`] (which carries the §4 exit-code class).
//!
//! | condition                    | `SmahError` variant     | exit |
//! |------------------------------|-------------------------|------|
//! | missing key                  | `MissingApiKey`         | 4 (row 6 names `<PROVIDER>_API_KEY`) |
//! | 401 / 403 rejected key       | `ProviderAuth`          | 4    |
//! | 4xx non-auth (incl. 429 rate limit) | `ProviderRequest` | 5    |
//! | 5xx                          | `ProviderUnavailable`   | 5    |
//! | network failure / timeout    | `Network`               | 5    |
//! | unparseable success body     | `Internal`              | 1    |
//!
//! Unknown provider key and malformed settings block (both exit 3) are
//! raised directly by [registry](crate::registry) /
//! [credentials](crate::credentials), not here.
//!
//! The provider named in every message is the config key (`"anthropic"`,
//! `"openai"`), matching §4's `<provider>`.

use serde_json::Value;
use smah_core::errors::SmahError;

/// Longest `<detail>` excerpt kept before truncation (§4 detail is one line).
const DETAIL_MAX_CHARS: usize = 300;

/// Translate a non-success HTTP status into the §4 class.
pub(crate) fn http_error(provider: &str, status: reqwest::StatusCode, body: &str) -> SmahError {
    let code = status.as_u16();
    match code {
        // Rejected key: §4 row 8, exit 4.
        401 | 403 => SmahError::ProviderAuth {
            provider: provider.to_string(),
            code,
        },
        // Any other 4xx — includes 429 rate limiting: §4 row 9, exit 5.
        400..=499 => SmahError::ProviderRequest {
            provider: provider.to_string(),
            code,
            detail: error_detail(body),
        },
        // 5xx: §4 row 10, exit 5.
        500..=599 => SmahError::ProviderUnavailable {
            provider: provider.to_string(),
            code,
        },
        // Unexpected status class (1xx/3xx): treated as a request failure.
        _ => SmahError::ProviderRequest {
            provider: provider.to_string(),
            code,
            detail: error_detail(body),
        },
    }
}

/// Translate a transport failure (DNS, refused, timeout) into §4 row 11.
pub(crate) fn transport_error(provider: &str, err: &reqwest::Error) -> SmahError {
    SmahError::Network {
        provider: provider.to_string(),
        detail: one_line(&err.to_string()),
    }
}

/// Provider message for an error body, one line, truncated.
///
/// Both vendors nest the human message at `error.message`; the raw body is
/// the fallback for non-JSON or oddly-shaped errors.
pub(crate) fn error_detail(body: &str) -> String {
    let extracted = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .or_else(|| value.get("error").and_then(Value::as_str))
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.to_string());
    truncate(&one_line(&extracted))
}

/// Collapse whitespace to a single line (§4: `<detail>` is one line).
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Cap a detail string, ASCII ellipsis when cut.
fn truncate(text: &str) -> String {
    if text.chars().count() <= DETAIL_MAX_CHARS {
        return text.to_string();
    }
    let cut: String = text.chars().take(DETAIL_MAX_CHARS).collect();
    format!("{cut}...")
}

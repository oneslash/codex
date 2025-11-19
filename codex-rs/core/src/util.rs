use std::borrow::Cow;
use std::sync::OnceLock;
use std::time::Duration;

use rand::Rng;
use regex_lite::Regex;
use tracing::debug;
use tracing::error;

const INITIAL_DELAY_MS: u64 = 200;
const BACKOFF_FACTOR: f64 = 2.0;

pub(crate) fn backoff(attempt: u64) -> Duration {
    let exp = BACKOFF_FACTOR.powi(attempt.saturating_sub(1) as i32);
    let base = (INITIAL_DELAY_MS as f64 * exp) as u64;
    let jitter = rand::rng().random_range(0.9..1.1);
    Duration::from_millis((base as f64 * jitter) as u64)
}

pub(crate) fn error_or_panic(message: String) {
    if cfg!(debug_assertions) || env!("CARGO_PKG_VERSION").contains("alpha") {
        panic!("{message}");
    } else {
        error!("{message}");
    }
}

pub(crate) fn try_parse_error_message(text: &str) -> String {
    debug!("Parsing server error response: {}", text);
    let json = serde_json::from_str::<serde_json::Value>(text).unwrap_or_default();
    if let Some(error) = json.get("error")
        && let Some(message) = error.get("message")
        && let Some(message_str) = message.as_str()
    {
        return message_str.to_string();
    }
    if text.is_empty() {
        return "Unknown error".to_string();
    }
    text.to_string()
}

/// Strip model-emitted citation markup so it does not leak into user-visible text.
///
/// Handles both private-use-wrapped blocks (e.g., `citeturn2`) and
/// angle-bracket forms (`<cite|path:line|>`). Returns a borrowed `Cow` when
/// nothing changes to avoid allocations on the hot path.
pub fn strip_citation_markup(text: &str) -> Cow<'_, str> {
    static PUA_RE: OnceLock<Regex> = OnceLock::new();
    static ANGLE_RE: OnceLock<Regex> = OnceLock::new();

    let re_pua = PUA_RE.get_or_init(|| {
        Regex::new(r"\u{e200}cite[\s\S]*?\u{e201}")
            .unwrap_or_else(|_| panic!("invalid citation regex"))
    });
    let re_angle = ANGLE_RE.get_or_init(|| {
        Regex::new(r"<cite\|([\s\S]*?)\|>")
            .unwrap_or_else(|_| panic!("invalid angle citation regex"))
    });

    if re_pua.find(text).is_some() {
        let replaced = re_pua.replace_all(text, "");
        if re_angle.is_match(replaced.as_ref()) {
            Cow::Owned(re_angle.replace_all(replaced.as_ref(), "$1").into_owned())
        } else {
            Cow::Owned(replaced.into_owned())
        }
    } else if re_angle.is_match(text) {
        Cow::Owned(re_angle.replace_all(text, "$1").into_owned())
    } else {
        Cow::Borrowed(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_try_parse_error_message() {
        let text = r#"{
  "error": {
    "message": "Your refresh token has already been used to generate a new access token. Please try signing in again.",
    "type": "invalid_request_error",
    "param": null,
    "code": "refresh_token_reused"
  }
}"#;
        let message = try_parse_error_message(text);
        assert_eq!(
            message,
            "Your refresh token has already been used to generate a new access token. Please try signing in again."
        );
    }

    #[test]
    fn test_try_parse_error_message_no_error() {
        let text = r#"{"message": "test"}"#;
        let message = try_parse_error_message(text);
        assert_eq!(message, r#"{"message": "test"}"#);
    }

    #[test]
    fn strip_citation_markup_removes_private_use_block() {
        let src = "Hello citeturn2search0 world";
        let out = strip_citation_markup(src);
        assert_eq!(out, "Hello  world");
    }

    #[test]
    fn strip_citation_markup_unwraps_angle_block() {
        let src = "See <cite|web/src/foo.svelte:1|> for details";
        let out = strip_citation_markup(src);
        assert_eq!(out, "See web/src/foo.svelte:1 for details");
    }
}

/// Simple locale module — replaces rust-i18n for Rust-side messages.
///
/// rust-i18n v4.0.0 fails silently on Windows because normpath converts
/// the locales directory to a Windows path, which globwalk cannot glob
/// against correctly.  This module provides the same interface with a
/// plain match-table approach that is guaranteed to work on all platforms.
use std::sync::atomic::{AtomicBool, Ordering};

static IS_JA: AtomicBool = AtomicBool::new(false);

pub fn set_locale(lang: &str) {
    IS_JA.store(lang == "ja", Ordering::SeqCst);
}

/// Look up a translation key with optional `%{name}` interpolation.
///
/// Returns the English string when the locale is not Japanese, or when the
/// key is not found in either table (returns the key itself as fallback).
pub fn tr(key: &str, params: &[(&str, &str)]) -> String {
    let template = if IS_JA.load(Ordering::SeqCst) {
        ja(key).unwrap_or(key)
    } else {
        en(key).unwrap_or(key)
    };
    if params.is_empty() {
        return template.to_string();
    }
    let mut s = template.to_string();
    for (name, val) in params {
        s = s.replace(&format!("%{{{name}}}"), val);
    }
    s
}

fn en(key: &str) -> Option<&'static str> {
    Some(match key {
        "status.running" => "Running\u{2026}",
        "status.query_finished" => "%{ms} ms  \u{00b7}  %{rows} rows",
        "status.cancelled" => "Cancelled",
        "status.error" => "Error: %{msg}",
        "status.disconnected" => "Disconnected: %{id}",
        "status.not_connected" => "Not connected",
        "status.connect_failed" => "Connection failed: %{msg}",
        "status.metadata_unavailable" => "Metadata unavailable: %{msg}",
        "error.no_active_connection" => "No active connection",
        "error.db_connect_failed" => "Failed to connect: %{reason}",
        "error.query_failed" => "Query error: %{reason}",
        "error.query_cancelled" => "Query cancelled",
        "error.db_error" => "Database error: %{reason}",
        _ => return None,
    })
}

fn ja(key: &str) -> Option<&'static str> {
    Some(match key {
        "status.running" => "\u{5b9f}\u{884c}\u{4e2d}\u{2026}",
        "status.query_finished" => "%{ms} ms  \u{00b7}  %{rows} \u{884c}",
        "status.cancelled" => {
            "\u{30ad}\u{30e3}\u{30f3}\u{30bb}\u{30eb}\u{3057}\u{307e}\u{3057}\u{305f}"
        }
        "status.error" => "\u{30a8}\u{30e9}\u{30fc}: %{msg}",
        "status.disconnected" => "\u{5207}\u{65ad}\u{3057}\u{307e}\u{3057}\u{305f}: %{id}",
        "status.not_connected" => "\u{672a}\u{63a5}\u{7d9a}",
        "status.connect_failed" => {
            "\u{63a5}\u{7d9a}\u{306b}\u{5931}\u{6557}\u{3057}\u{307e}\u{3057}\u{305f}: %{msg}"
        }
        "status.metadata_unavailable" => {
            "\u{30e1}\u{30bf}\u{30c7}\u{30fc}\u{30bf}\u{3092}\u{53d6}\u{5f97}\u{3067}\u{304d}\u{307e}\u{305b}\u{3093}\u{3067}\u{3057}\u{305f}: %{msg}"
        }
        "error.no_active_connection" => {
            "\u{63a5}\u{7d9a}\u{304c}\u{3042}\u{308a}\u{307e}\u{305b}\u{3093}"
        }
        "error.db_connect_failed" => {
            "\u{63a5}\u{7d9a}\u{306b}\u{5931}\u{6557}\u{3057}\u{307e}\u{3057}\u{305f}: %{reason}"
        }
        "error.query_failed" => "\u{30af}\u{30a8}\u{30ea}\u{30a8}\u{30e9}\u{30fc}: %{reason}",
        "error.query_cancelled" => {
            "\u{30af}\u{30a8}\u{30ea}\u{3092}\u{30ad}\u{30e3}\u{30f3}\u{30bb}\u{30eb}\u{3057}\u{307e}\u{3057}\u{305f}"
        }
        "error.db_error" => {
            "\u{30c7}\u{30fc}\u{30bf}\u{30d9}\u{30fc}\u{30b9}\u{30a8}\u{30e9}\u{30fc}: %{reason}"
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tr_should_return_english_by_default() {
        set_locale("en");
        assert_eq!(tr("status.running", &[]), "Running\u{2026}");
        assert_eq!(
            tr("error.no_active_connection", &[]),
            "No active connection"
        );
    }

    #[test]
    fn tr_should_interpolate_params() {
        set_locale("en");
        let result = tr("error.db_error", &[("reason", "conn refused")]);
        assert_eq!(result, "Database error: conn refused");
    }

    #[test]
    fn tr_should_return_japanese_when_locale_is_ja() {
        set_locale("ja");
        assert_eq!(
            tr("status.cancelled", &[]),
            "\u{30ad}\u{30e3}\u{30f3}\u{30bb}\u{30eb}\u{3057}\u{307e}\u{3057}\u{305f}"
        );
        assert_eq!(tr("status.not_connected", &[]), "\u{672a}\u{63a5}\u{7d9a}");
        set_locale("en"); // restore
    }

    #[test]
    fn tr_should_return_key_for_unknown_keys() {
        set_locale("en");
        assert_eq!(tr("nonexistent.key", &[]), "nonexistent.key");
    }
}

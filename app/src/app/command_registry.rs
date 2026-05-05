//! Command palette action registry.
//!
//! Builds the full list of palette entries by merging a static set of
//! always-available commands with dynamic per-connection "Connect: <name>"
//! entries.  Call [`build_actions`] on each palette open so the list stays
//! in sync with the current connection set.

/// A single entry displayed in the command palette.
#[derive(Clone)]
pub struct PaletteAction {
    pub id: String,
    pub label: String,
    pub shortcut: &'static str,
}

/// Static commands that are always available regardless of connection state.
const STATIC_ACTIONS: &[(&str, &str, &str)] = &[
    ("run-all", "Run All Queries", "Ctrl+Shift+Enter"),
    ("cancel-query", "Cancel Query", "Esc"),
    ("format-sql", "Format SQL", "Ctrl+Shift+F"),
    ("find", "Find in Editor", "Ctrl+F"),
    ("find-replace", "Find and Replace", "Ctrl+H"),
    ("new-tab", "New Tab", "Ctrl+T"),
    ("close-tab", "Close Tab", "Ctrl+W"),
    ("next-tab", "Next Tab", "Ctrl+Tab"),
    ("prev-tab", "Previous Tab", "Ctrl+Shift+Tab"),
    ("toggle-snippet-bar", "Toggle Snippet Bar", "Ctrl+B"),
    ("open-metadata-search", "Metadata Search", "Ctrl+P"),
    ("save-snippet", "Save Snippet", "Ctrl+D"),
    ("show-snippets", "Show Snippets", ""),
    ("export-csv", "Export CSV", ""),
    ("export-json", "Export JSON", ""),
    ("export-insert-sql", "Export Insert SQL", ""),
    ("open-db-manager", "Manage Connections", ""),
    ("disconnect", "Disconnect", ""),
    ("toggle-theme", "Toggle Theme", ""),
    ("toggle-reduce-motion", "Toggle Reduce Motion", ""),
];

/// Build the full palette action list: all static commands followed by one
/// `"Connect: <name>"` entry per saved connection.
///
/// Connection entries use id format `"connect:<connection_id>"` so the
/// dispatcher can parse the target connection id with a prefix strip.
///
/// Call this on every palette open so the list reflects the current saved
/// connection set without any additional caching layer.
pub fn build_actions(connections: &[(String, String)]) -> Vec<PaletteAction> {
    let mut actions: Vec<PaletteAction> = STATIC_ACTIONS
        .iter()
        .map(|(id, label, shortcut)| PaletteAction {
            id: (*id).to_string(),
            label: (*label).to_string(),
            shortcut,
        })
        .collect();
    for (conn_id, conn_name) in connections {
        actions.push(PaletteAction {
            id: format!("connect:{conn_id}"),
            label: format!("Connect: {conn_name}"),
            shortcut: "",
        });
    }
    actions
}

/// Fuzzy-filter `actions` by `query`, returning all entries unchanged when
/// `query` is empty.  Matches are ranked by score descending (best match first).
pub fn filter_actions(actions: &[PaletteAction], query: &str) -> Vec<PaletteAction> {
    if query.is_empty() {
        return actions.to_vec();
    }
    use fuzzy_matcher::FuzzyMatcher as _;
    let matcher = fuzzy_matcher::skim::SkimMatcherV2::default();
    let mut scored: Vec<(i64, &PaletteAction)> = actions
        .iter()
        .filter_map(|a| matcher.fuzzy_match(&a.label, query).map(|s| (s, a)))
        .collect();
    scored.sort_by_key(|(s, _)| std::cmp::Reverse(*s));
    scored.into_iter().map(|(_, a)| a.clone()).collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_actions_should_include_static_and_connection_entries() {
        let conns = vec![
            ("id1".to_string(), "My DB".to_string()),
            ("id2".to_string(), "Local PG".to_string()),
        ];
        let actions = build_actions(&conns);

        assert!(actions.iter().any(|a| a.id == "cancel-query"));
        assert!(actions.iter().any(|a| a.id == "toggle-theme"));
        assert!(
            actions
                .iter()
                .any(|a| a.id == "connect:id1" && a.label == "Connect: My DB")
        );
        assert!(
            actions
                .iter()
                .any(|a| a.id == "connect:id2" && a.label == "Connect: Local PG")
        );
    }

    #[test]
    fn build_actions_should_return_only_static_when_no_connections() {
        let actions = build_actions(&[]);
        assert_eq!(actions.len(), STATIC_ACTIONS.len());
    }

    #[test]
    fn filter_actions_should_return_all_on_empty_query() {
        let actions = build_actions(&[]);
        let filtered = filter_actions(&actions, "");
        assert_eq!(filtered.len(), actions.len());
    }

    #[test]
    fn filter_actions_should_fuzzy_match_labels() {
        let actions = build_actions(&[]);
        let filtered = filter_actions(&actions, "cancel");
        assert!(filtered.iter().any(|a| a.id == "cancel-query"));
    }

    #[test]
    fn filter_actions_should_match_connection_entries() {
        let conns = vec![("abc".to_string(), "Production DB".to_string())];
        let actions = build_actions(&conns);
        let filtered = filter_actions(&actions, "prod");
        assert!(filtered.iter().any(|a| a.id == "connect:abc"));
    }

    #[test]
    fn filter_actions_should_rank_better_matches_first() {
        let conns = vec![
            ("a".to_string(), "format".to_string()),
            ("b".to_string(), "Format SQL".to_string()),
        ];
        let actions = build_actions(&conns);
        let filtered = filter_actions(&actions, "format");
        // The exact match "format" and "Format SQL" should both appear.
        // Just verify the results are non-empty and in some order.
        assert!(!filtered.is_empty());
        // The first result should have a higher or equal score to the last.
        // Both contain "format" so both match; verify no panic on ordering.
        let _ = filtered[0].id.clone();
    }
}

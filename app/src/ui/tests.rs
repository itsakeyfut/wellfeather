use super::*;
use wf_config::models::{ConnectionConfig, DbTypeName};
use wf_db::models::{DbMetadata, TableInfo};

// ── find_prefix_start ────────────────────────────────────────────────────────

#[test]
fn find_prefix_start_should_return_word_start_before_cursor() {
    assert_eq!(find_prefix_start("SELECT sel", 10), 7);
}

#[test]
fn find_prefix_start_should_return_cursor_when_at_space() {
    assert_eq!(find_prefix_start("SELECT ", 7), 7);
}

#[test]
fn find_prefix_start_should_return_after_dot_for_qualified_name() {
    assert_eq!(find_prefix_start("u.em", 4), 2);
}

#[test]
fn find_prefix_start_should_return_cursor_when_no_prefix() {
    assert_eq!(find_prefix_start("SELECT * FROM ", 14), 14);
}

// ── sql_has_from ─────────────────────────────────────────────────────────────

#[test]
fn sql_has_from_should_return_true_when_from_present() {
    assert!(sql_has_from("SELECT id FROM users"));
}

#[test]
fn sql_has_from_should_return_false_when_no_from() {
    assert!(!sql_has_from("SELECT id, name"));
}

#[test]
fn sql_has_from_should_return_true_for_multiline_from() {
    assert!(sql_has_from("SELECT id\nFROM users"));
}

#[test]
fn sql_has_from_should_return_false_for_from_in_column_name() {
    // "from" inside a word like "transform" should not match
    assert!(!sql_has_from("SELECT transform_id"));
}

// ── is_terminal_expression ────────────────────────────────────────────────────

#[test]
fn is_terminal_expression_should_return_true_for_is_not_null() {
    assert!(is_terminal_expression(
        "SELECT name FROM users WHERE deleted_at IS NOT NULL"
    ));
    assert!(is_terminal_expression("WHERE col IS NULL"));
}

#[test]
fn is_terminal_expression_should_return_true_for_boolean_keywords() {
    assert!(is_terminal_expression("WHERE active = TRUE"));
    assert!(is_terminal_expression("WHERE active = FALSE"));
}

#[test]
fn is_terminal_expression_should_return_true_for_direction_keywords() {
    assert!(is_terminal_expression("ORDER BY id ASC"));
    assert!(is_terminal_expression("ORDER BY id DESC"));
}

#[test]
fn is_terminal_expression_should_return_true_for_string_literal() {
    assert!(is_terminal_expression("WHERE name = 'alice'"));
}

#[test]
fn is_terminal_expression_should_return_true_for_numeric_literal() {
    assert!(is_terminal_expression("WHERE id = 5"));
    assert!(is_terminal_expression("LIMIT 10"));
}

#[test]
fn is_terminal_expression_should_return_false_for_non_terminal_positions() {
    assert!(!is_terminal_expression("FROM users WHERE"));
    assert!(!is_terminal_expression("SELECT * FROM users"));
    assert!(!is_terminal_expression("SELECT id"));
}

#[test]
fn is_terminal_expression_should_not_match_word_ending_with_null_suffix() {
    // "nullify" last word is "NULLIFY" — not the keyword "NULL"
    assert!(!is_terminal_expression("WHERE nullify"));
    // "is_not_null_col" is a column name, not the keyword NULL
    assert!(!is_terminal_expression("SELECT is_not_null_col"));
}

fn make_conn(id: &str, name: &str) -> ConnectionConfig {
    ConnectionConfig {
        id: id.to_string(),
        name: name.to_string(),
        db_type: DbTypeName::SQLite,
        connection_string: None,
        host: None,
        port: None,
        user: None,
        password_encrypted: None,
        database: None,
        safe_dml: true,
        read_only: false,
    }
}

fn make_meta(tables: &[&str]) -> DbMetadata {
    DbMetadata {
        tables: tables
            .iter()
            .map(|n| TableInfo {
                name: n.to_string(),
                columns: vec![],
            })
            .collect(),
        views: vec![],
        stored_procs: vec![],
        indexes: vec![],
    }
}

#[test]
fn build_sidebar_tree_should_render_connection_nodes() {
    let conns = vec![make_conn("a", "Alpha"), make_conn("b", "Beta")];
    let nodes = build_sidebar_tree(
        &conns,
        "",
        &HashMap::new(),
        &HashSet::new(),
        &HashMap::new(),
    );
    assert_eq!(nodes.len(), 2);
    assert_eq!(nodes[0].label.as_str(), "Alpha");
    assert_eq!(nodes[0].level, 0);
    assert_eq!(nodes[0].node_kind.as_str(), "connection");
    assert_eq!(nodes[1].label.as_str(), "Beta");
}

#[test]
fn build_sidebar_tree_should_show_categories_when_connection_expanded() {
    let conns = vec![make_conn("a", "Alpha")];
    let mut expanded = HashSet::new();
    expanded.insert("conn:a".to_string());
    let mut metadata = HashMap::new();
    metadata.insert("a".to_string(), make_meta(&["users"]));
    let nodes = build_sidebar_tree(&conns, "a", &metadata, &expanded, &HashMap::new());
    // conn + Tables + users(visible=false) + Views + Stored Procedures + Indexes = 6 nodes
    // Children are always emitted; visible flag drives animation.
    assert_eq!(nodes.len(), 6);
    assert_eq!(nodes[1].label.as_str(), "Tables");
    assert_eq!(nodes[1].level, 1);
    assert_eq!(nodes[1].node_kind.as_str(), "category");
    // "users" is emitted but invisible (Tables category not expanded)
    assert_eq!(nodes[2].label.as_str(), "users");
    assert!(!nodes[2].visible);
}

#[test]
fn build_sidebar_tree_should_show_items_when_category_expanded() {
    let conns = vec![make_conn("a", "Alpha")];
    let mut expanded = HashSet::new();
    expanded.insert("conn:a".to_string());
    expanded.insert("cat:a:Tables".to_string());
    let mut metadata = HashMap::new();
    metadata.insert("a".to_string(), make_meta(&["users", "orders"]));
    let nodes = build_sidebar_tree(&conns, "a", &metadata, &expanded, &HashMap::new());
    // conn + Tables + users + orders + Views + Stored Procedures + Indexes = 7
    assert_eq!(nodes.len(), 7);
    assert_eq!(nodes[2].label.as_str(), "users");
    assert_eq!(nodes[2].level, 2);
    assert_eq!(nodes[2].node_kind.as_str(), "table");
    assert_eq!(nodes[3].label.as_str(), "orders");
}

#[test]
fn build_sidebar_tree_should_hide_children_when_collapsed() {
    let conns = vec![make_conn("a", "Alpha")];
    let nodes = build_sidebar_tree(
        &conns,
        "a",
        &HashMap::new(),
        &HashSet::new(),
        &HashMap::new(),
    );
    // No metadata → no child nodes emitted at all.
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].level, 0);
}

#[test]
fn build_sidebar_tree_should_emit_invisible_children_for_animation() {
    let conns = vec![make_conn("a", "Alpha")];
    let mut metadata = HashMap::new();
    metadata.insert("a".to_string(), make_meta(&["users"]));
    // Connection collapsed (not in expanded)
    let nodes = build_sidebar_tree(&conns, "a", &metadata, &HashSet::new(), &HashMap::new());
    // Categories are emitted but invisible
    assert!(nodes.len() > 1);
    for node in nodes.iter().skip(1) {
        assert!(
            !node.visible,
            "category should be invisible when conn collapsed"
        );
    }
}

#[test]
fn build_sidebar_tree_should_mark_active_connection() {
    let conns = vec![make_conn("a", "Alpha"), make_conn("b", "Beta")];
    let nodes = build_sidebar_tree(
        &conns,
        "b",
        &HashMap::new(),
        &HashSet::new(),
        &HashMap::new(),
    );
    assert!(!nodes[0].is_active);
    assert!(nodes[1].is_active);
}

// ── filter_rows tests ─────────────────────────────────────────────────────────

fn ss(s: &str) -> slint::SharedString {
    s.into()
}

fn sv(s: &str) -> Option<String> {
    Some(s.to_string())
}

#[test]
fn filter_rows_should_return_all_when_query_empty() {
    let cols = vec![ss("id"), ss("name")];
    let rows = vec![vec![sv("1"), sv("Alice")], vec![sv("2"), sv("Bob")]];
    assert_eq!(filter_rows(&cols, &rows, "").len(), 2);
    assert_eq!(filter_rows(&cols, &rows, "   ").len(), 2);
}

#[test]
fn filter_rows_should_match_substring_across_all_columns() {
    let cols = vec![ss("name"), ss("city")];
    let rows = vec![
        vec![sv("Alice"), sv("Tokyo")],
        vec![sv("Bob"), sv("Osaka")],
        vec![sv("Alice Smith"), sv("Kyoto")],
    ];
    let result = filter_rows(&cols, &rows, "alice");
    assert_eq!(result.len(), 2);
    assert_eq!(result[0][0].as_deref(), Some("Alice"));
    assert_eq!(result[1][0].as_deref(), Some("Alice Smith"));
}

#[test]
fn filter_rows_should_match_exact_column_value() {
    let cols = vec![ss("name"), ss("city")];
    let rows = vec![vec![sv("Alice"), sv("Tokyo")], vec![sv("Bob"), sv("Osaka")]];
    let result = filter_rows(&cols, &rows, "city = 'Tokyo'");
    assert_eq!(result.len(), 1);
    assert_eq!(result[0][1].as_deref(), Some("Tokyo"));
}

#[test]
fn filter_rows_should_return_empty_when_column_not_found() {
    let cols = vec![ss("name")];
    let rows = vec![vec![sv("Alice")]];
    let result = filter_rows(&cols, &rows, "missing = 'x'");
    assert!(result.is_empty());
}

#[test]
fn filter_rows_should_not_match_null_with_eq_predicate() {
    let cols = vec![ss("name")];
    let rows = vec![vec![None], vec![sv("Alice")]];
    let result = filter_rows(&cols, &rows, "name = ''");
    // NULL != '' — only the non-null empty string row should match, but here
    // there is none, so result is empty.
    assert!(result.is_empty());
}

#[test]
fn filter_rows_should_treat_null_as_empty_for_substring_match() {
    let cols = vec![ss("name")];
    // NULL treated as "" for substring search — empty query prefix matches all.
    let rows = vec![vec![None], vec![sv("Alice")]];
    // Substring "" matches everything (but we trim, so empty query returns all).
    let result = filter_rows(&cols, &rows, "Alice");
    assert_eq!(result.len(), 1);
    assert_eq!(result[0][0].as_deref(), Some("Alice"));
}

// ── sort_rows tests ───────────────────────────────────────────────────────────

#[test]
fn sort_rows_should_sort_strings_ascending() {
    let mut rows = vec![vec![sv("banana")], vec![sv("apple")], vec![sv("cherry")]];
    sort_rows(&mut rows, 0, true);
    assert_eq!(rows[0][0].as_deref(), Some("apple"));
    assert_eq!(rows[1][0].as_deref(), Some("banana"));
    assert_eq!(rows[2][0].as_deref(), Some("cherry"));
}

#[test]
fn sort_rows_should_sort_strings_descending() {
    let mut rows = vec![vec![sv("banana")], vec![sv("apple")], vec![sv("cherry")]];
    sort_rows(&mut rows, 0, false);
    assert_eq!(rows[0][0].as_deref(), Some("cherry"));
    assert_eq!(rows[1][0].as_deref(), Some("banana"));
    assert_eq!(rows[2][0].as_deref(), Some("apple"));
}

#[test]
fn sort_rows_should_sort_numerically_when_values_are_numbers() {
    let mut rows = vec![vec![sv("10")], vec![sv("2")], vec![sv("20")]];
    sort_rows(&mut rows, 0, true);
    assert_eq!(rows[0][0].as_deref(), Some("2"));
    assert_eq!(rows[1][0].as_deref(), Some("10"));
    assert_eq!(rows[2][0].as_deref(), Some("20"));
}

#[test]
fn sort_rows_should_put_nulls_last_ascending() {
    let mut rows = vec![vec![None], vec![sv("b")], vec![sv("a")]];
    sort_rows(&mut rows, 0, true);
    assert_eq!(rows[0][0].as_deref(), Some("a"));
    assert_eq!(rows[1][0].as_deref(), Some("b"));
    assert!(rows[2][0].is_none());
}

#[test]
fn sort_rows_should_put_nulls_last_descending() {
    let mut rows = vec![vec![None], vec![sv("b")], vec![sv("a")]];
    sort_rows(&mut rows, 0, false);
    assert_eq!(rows[0][0].as_deref(), Some("b"));
    assert_eq!(rows[1][0].as_deref(), Some("a"));
    assert!(rows[2][0].is_none());
}

// ── cells_to_tsv / result_to_tsv tests ───────────────────────────────────────

#[test]
fn cells_to_tsv_should_join_values_with_tabs() {
    let cells = vec![sv("a"), sv("b"), sv("c")];
    assert_eq!(cells_to_tsv(&cells), "a\tb\tc");
}

#[test]
fn cells_to_tsv_should_render_null_as_empty_string() {
    let cells = vec![sv("a"), None, sv("c")];
    assert_eq!(cells_to_tsv(&cells), "a\t\tc");
}

#[test]
fn cells_to_tsv_should_handle_empty_row() {
    let cells: Vec<Option<String>> = vec![];
    assert_eq!(cells_to_tsv(&cells), "");
}

#[test]
fn result_to_tsv_should_include_header_and_rows() {
    let cols = vec!["id", "name"];
    let rows = vec![vec![sv("1"), sv("Alice")], vec![sv("2"), sv("Bob")]];
    let tsv = result_to_tsv(&cols, &rows);
    assert_eq!(tsv, "id\tname\n1\tAlice\n2\tBob");
}

#[test]
fn result_to_tsv_should_render_null_cells_as_empty_string() {
    let cols = vec!["id", "name"];
    let rows = vec![vec![sv("1"), None]];
    let tsv = result_to_tsv(&cols, &rows);
    assert_eq!(tsv, "id\tname\n1\t");
}

#[test]
fn result_to_tsv_should_produce_header_only_when_no_rows() {
    let cols = vec!["id", "name"];
    let rows: Vec<Vec<Option<String>>> = vec![];
    let tsv = result_to_tsv(&cols, &rows);
    assert_eq!(tsv, "id\tname");
}

// ── append_editor_text tests ──────────────────────────────────────────────────

#[test]
fn append_editor_text_should_set_text_when_editor_is_empty() {
    assert_eq!(append_editor_text("", "SELECT * FROM t"), "SELECT * FROM t");
}

#[test]
fn append_editor_text_should_prepend_newline_when_content_exists() {
    assert_eq!(
        append_editor_text("SELECT 1", "SELECT * FROM t"),
        "SELECT 1\nSELECT * FROM t"
    );
}

#[test]
fn append_editor_text_should_not_double_newline_when_content_ends_with_newline() {
    assert_eq!(
        append_editor_text("SELECT 1\n", "SELECT * FROM t"),
        "SELECT 1\nSELECT * FROM t"
    );
}

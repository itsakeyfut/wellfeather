//! Cursor-position context analysis for SQL completion.
//!
//! [`parse_context`] inspects the SQL text and cursor position to determine
//! whether the cursor is in a keyword, table-name, or column-name context.

/// Describes what kind of completion is appropriate at the cursor position.
#[derive(Debug, Clone, PartialEq)]
pub enum CompletionContext {
    /// The cursor is at a position where a SQL keyword is expected.
    Keyword,
    /// The cursor follows `FROM` or `JOIN` — a table/view name is expected.
    TableName,
    /// The cursor is in a column position (after `SELECT`, `WHERE`, etc.).
    ///
    /// `table` carries the first table name found in the `FROM` clause,
    /// resolved through alias lookup when the cursor is in `alias.column` notation.
    ColumnName { table: Option<String> },
    /// Cursor is at a space after a complete table/view name in a FROM clause — suggest
    /// next-clause keywords (WHERE, JOIN, ORDER BY, LIMIT, etc.) ranked by frequency.
    NextClause,
    /// Cursor is at a space after a complete table/view name in a JOIN clause — suggest `ON`.
    JoinOn,
    /// Cursor is at a space after a column reference in WHERE / HAVING / ON — suggest
    /// comparison operators (`=`, `!=`, `IS NULL`, `LIKE`, …).
    Operator,
    /// Cursor follows `ON` with nothing typed yet — suggest table names from FROM/JOIN clauses
    /// so the user can continue with dot-notation column access.
    JoinConditionTable { tables: Vec<String> },
    /// Cursor follows a comparison operator (`=`, `<`, `>`) — suggest value literals
    /// (`''`, `NULL`, `TRUE`, `FALSE`).
    ValueExpected,
    /// Context cannot be determined (e.g. the SQL is empty).
    None,
}

/// Determine the completion context for `sql` at byte offset `cursor_pos`.
///
/// The `|` in the examples below marks the cursor position:
///
/// ```text
/// "SELECT |"                → Keyword
/// "SELECT * FROM |"         → TableName
/// "SELECT | FROM users"     → ColumnName { table: Some("users") }
/// "SELECT u.| FROM users u" → ColumnName { table: Some("users") }
/// ```
pub fn parse_context(sql: &str, cursor_pos: usize) -> CompletionContext {
    let cursor_pos = cursor_pos.min(sql.len());
    let before = &sql[..cursor_pos];

    // After a semicolon there is nothing more to complete in this statement.
    if before.trim_end().ends_with(';') {
        return CompletionContext::None;
    }

    // Dot notation: "alias.|" → column completion scoped to that alias's table.
    if let Some(prefix) = before.trim_end().strip_suffix('.') {
        let alias = last_ident(prefix);
        let table = resolve_alias(sql, alias);
        return CompletionContext::ColumnName { table };
    }

    if before.trim().is_empty() {
        return CompletionContext::None;
    }

    let before_upper = before.to_ascii_uppercase();
    match last_trigger_keyword(&before_upper) {
        Some(trig @ ("FROM" | "JOIN" | "INTO")) => {
            // After a complete table identifier (cursor at space): distinguish FROM→NextClause
            // from JOIN→JoinOn so the popup stays contextually minimal.
            if before.ends_with(|c: char| c.is_ascii_whitespace()) {
                let trigger_end = last_kw_pos(&before_upper, trig)
                    .map(|p| p + trig.len())
                    .unwrap_or(0);
                let after_trigger = before[trigger_end..].trim_start();
                let first_token_len = after_trigger
                    .find(|c: char| !c.is_alphanumeric() && c != '_')
                    .unwrap_or(after_trigger.len());
                if first_token_len > 0 {
                    return if trig == "JOIN" {
                        CompletionContext::JoinOn
                    } else {
                        CompletionContext::NextClause
                    };
                }
            }
            CompletionContext::TableName
        }
        Some(trig @ ("WHERE" | "HAVING" | "ON")) => {
            if before.ends_with(|c: char| c.is_ascii_whitespace()) {
                let trigger_end = last_kw_pos(&before_upper, trig)
                    .map(|p| p + trig.len())
                    .unwrap_or(0);
                let after_trigger = before[trigger_end..].trim();

                // ON with nothing typed yet → suggest referenced table names for dot notation
                if trig == "ON" && after_trigger.is_empty() {
                    let tables = extract_referenced_tables(sql);
                    if !tables.is_empty() {
                        return CompletionContext::JoinConditionTable { tables };
                    }
                }

                // Cursor follows a comparison operator → suggest value literals
                if after_trigger.ends_with(['=', '<', '>']) {
                    return CompletionContext::ValueExpected;
                }

                // Lone column/qualified-column identifier → suggest comparison operators
                if !after_trigger.is_empty()
                    && after_trigger
                        .chars()
                        .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
                {
                    return CompletionContext::Operator;
                }
            }
            match extract_from_table(sql) {
                Some(t) => CompletionContext::ColumnName { table: Some(t) },
                None => CompletionContext::Keyword,
            }
        }
        Some("SELECT") | Some("SET") | Some("BY") => CompletionContext::ColumnName {
            table: extract_from_table(sql),
        },
        _ => CompletionContext::Keyword,
    }
}

/// Extract the first table name after the `FROM` keyword in `sql`.
///
/// Returns `None` if no `FROM` clause is present or the clause is empty.
pub fn extract_from_table(sql: &str) -> Option<String> {
    let upper = sql.to_ascii_uppercase();
    let from_pos = first_kw_pos(&upper, "FROM")?;
    let after = sql[from_pos + 4..].trim_start();
    let name: String = after
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() { None } else { Some(name) }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn is_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Find the last position where `kw` appears as a complete word in `upper_text`.
fn last_kw_pos(upper_text: &str, kw: &str) -> Option<usize> {
    let bytes = upper_text.as_bytes();
    let kw_len = kw.len();
    let mut found = None;
    for i in 0..upper_text.len() {
        if upper_text[i..].starts_with(kw) {
            let before_ok = i == 0 || !is_word_char(bytes[i - 1]);
            let after_pos = i + kw_len;
            let after_ok = after_pos >= upper_text.len() || !is_word_char(bytes[after_pos]);
            if before_ok && after_ok {
                found = Some(i);
            }
        }
    }
    found
}

/// Find the first position where `kw` appears as a complete word in `upper_text`.
fn first_kw_pos(upper_text: &str, kw: &str) -> Option<usize> {
    let bytes = upper_text.as_bytes();
    let kw_len = kw.len();
    for i in 0..upper_text.len() {
        if upper_text[i..].starts_with(kw) {
            let before_ok = i == 0 || !is_word_char(bytes[i - 1]);
            let after_pos = i + kw_len;
            let after_ok = after_pos >= upper_text.len() || !is_word_char(bytes[after_pos]);
            if before_ok && after_ok {
                return Some(i);
            }
        }
    }
    None
}

/// Among all trigger keywords, return the one with the highest (last) position in `upper_text`.
fn last_trigger_keyword(upper_text: &str) -> Option<&'static str> {
    const TRIGGERS: &[&str] = &[
        "SELECT", "FROM", "JOIN", "WHERE", "SET", "HAVING", "INTO",
        "BY", // ORDER BY, GROUP BY → column names
        "ON", // JOIN ... ON → column names
    ];
    let mut best: Option<(usize, &'static str)> = None;
    for &kw in TRIGGERS {
        if let Some(pos) = last_kw_pos(upper_text, kw)
            && best.is_none_or(|(p, _)| pos > p)
        {
            best = Some((pos, kw));
        }
    }
    best.map(|(_, kw)| kw)
}

/// Return the last SQL identifier (alphanumeric + underscore) found in `s`.
fn last_ident(s: &str) -> Option<&str> {
    let end = s.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '_');
    if end.is_empty() {
        return None;
    }
    let start = end
        .rfind(|c: char| !c.is_alphanumeric() && c != '_')
        .map(|i| i + 1)
        .unwrap_or(0);
    Some(&end[start..])
}

/// Resolve `alias` to its actual table name by scanning the FROM clause of `sql`.
///
/// Handles `FROM tbl alias` and `FROM tbl AS alias` patterns.
/// Falls back to [`extract_from_table`] when the alias is not found.
fn resolve_alias(sql: &str, alias: Option<&str>) -> Option<String> {
    let alias = alias?;
    let alias_upper = alias.to_ascii_uppercase();
    let upper = sql.to_ascii_uppercase();

    let from_pos = first_kw_pos(&upper, "FROM")?;
    let clause_upper = &upper[from_pos + 4..];
    let clause_sql = &sql[from_pos + 4..];

    // Trim at the first stop keyword (WHERE, GROUP, ORDER, HAVING, LIMIT).
    const STOPS: &[&str] = &["WHERE", "GROUP", "ORDER", "HAVING", "LIMIT"];
    let end = STOPS
        .iter()
        .filter_map(|kw| first_kw_pos(clause_upper, kw))
        .min()
        .unwrap_or(clause_upper.len());

    let clause_upper = &clause_upper[..end];
    let clause_sql = &clause_sql[..end];

    let tokens_upper = ident_tokens(clause_upper);
    let tokens_sql = ident_tokens(clause_sql);

    for (i, &word_upper) in tokens_upper.iter().enumerate() {
        if word_upper.eq_ignore_ascii_case(&alias_upper) && i > 0 {
            let prev = tokens_upper[i - 1];
            if prev.eq_ignore_ascii_case("AS") {
                // "tablename AS alias" pattern
                if i >= 2 {
                    return Some(tokens_sql[i - 2].to_string());
                }
            } else if is_join_keyword(prev) {
                // "JOIN tablename" — no alias, the token itself IS the table name
                return Some(tokens_sql[i].to_string());
            } else {
                // "tablename alias" pattern
                return Some(tokens_sql[i - 1].to_string());
            }
        }
    }

    // Alias not matched — fall back to first table in FROM clause.
    extract_from_table(sql)
}

/// True if `kw` is a SQL keyword that can directly precede a table name or alias reference.
fn is_join_keyword(kw: &str) -> bool {
    matches!(
        kw.to_ascii_uppercase().as_str(),
        "FROM"
            | "JOIN"
            | "LEFT"
            | "RIGHT"
            | "INNER"
            | "OUTER"
            | "CROSS"
            | "FULL"
            | "NATURAL"
            | "ON"
    )
}

/// Returns `true` when `cursor_pos` is inside a SELECT column list.
///
/// Specifically: there is a `SELECT` keyword before the cursor with no `FROM`
/// keyword between that SELECT and the cursor position.
pub fn in_select_list(sql: &str, cursor_pos: usize) -> bool {
    let cursor_pos = cursor_pos.min(sql.len());
    let upper = sql.to_ascii_uppercase();
    let before_upper = &upper[..cursor_pos];
    let Some(select_pos) = last_kw_pos(before_upper, "SELECT") else {
        return false;
    };
    first_kw_pos(&before_upper[select_pos + 6..], "FROM").is_none()
}

/// Return the distinct table names referenced after `FROM` and `JOIN` keywords in `sql`.
///
/// Used to build `JoinConditionTable` candidates after `ON`.
pub fn extract_referenced_tables(sql: &str) -> Vec<String> {
    let upper = sql.to_ascii_uppercase();
    let mut tables = Vec::new();

    for kw in ["FROM", "JOIN"] {
        let mut start = 0;
        while start < upper.len() {
            let Some(rel) = upper[start..].find(kw) else {
                break;
            };
            let abs = start + rel;
            let bytes = upper.as_bytes();
            let before_ok = abs == 0 || !is_word_char(bytes[abs - 1]);
            let after_pos = abs + kw.len();
            let after_ok = after_pos >= upper.len() || !is_word_char(bytes[after_pos]);
            if before_ok && after_ok {
                let rest = sql[after_pos..].trim_start();
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() {
                    let name_upper = name.to_ascii_uppercase();
                    let is_keyword = matches!(
                        name_upper.as_str(),
                        "ON" | "WHERE"
                            | "HAVING"
                            | "GROUP"
                            | "ORDER"
                            | "LIMIT"
                            | "SET"
                            | "INNER"
                            | "LEFT"
                            | "RIGHT"
                            | "OUTER"
                            | "CROSS"
                            | "FULL"
                            | "NATURAL"
                    );
                    if !is_keyword && !tables.contains(&name) {
                        tables.push(name);
                    }
                }
            }
            start = abs + 1;
        }
    }

    tables
}

/// Split `text` into a list of identifier tokens (runs of alphanumeric + `_`).
fn ident_tokens(text: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut rest = text;
    loop {
        let s = rest.trim_start_matches(|c: char| !c.is_alphanumeric() && c != '_');
        if s.is_empty() {
            break;
        }
        let end = s
            .find(|c: char| !c.is_alphanumeric() && c != '_')
            .unwrap_or(s.len());
        tokens.push(&s[..end]);
        rest = &s[end..];
    }
    tokens
}

/// Returns `true` when `word` is a SQL keyword that should never be treated as
/// a column-name prefix in compound-prefix detection.
pub fn is_sql_keyword(word: &str) -> bool {
    matches!(
        word.to_ascii_uppercase().as_str(),
        "SELECT"
            | "FROM"
            | "WHERE"
            | "JOIN"
            | "INNER"
            | "LEFT"
            | "RIGHT"
            | "FULL"
            | "OUTER"
            | "CROSS"
            | "NATURAL"
            | "ON"
            | "AS"
            | "GROUP"
            | "ORDER"
            | "BY"
            | "HAVING"
            | "LIMIT"
            | "OFFSET"
            | "SET"
            | "INSERT"
            | "INTO"
            | "VALUES"
            | "UPDATE"
            | "DELETE"
            | "CREATE"
            | "DROP"
            | "ALTER"
            | "TABLE"
            | "INDEX"
            | "VIEW"
            | "WITH"
            | "DISTINCT"
            | "ALL"
            | "UNION"
            | "EXCEPT"
            | "INTERSECT"
            | "AND"
            | "OR"
            | "NOT"
            | "IN"
            | "EXISTS"
            | "LIKE"
            | "ILIKE"
            | "BETWEEN"
            | "IS"
            | "NULL"
            | "TRUE"
            | "FALSE"
            | "CASE"
            | "WHEN"
            | "THEN"
            | "ELSE"
            | "END"
            | "ASC"
            | "DESC"
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse `s` using the embedded `|` cursor marker.
    /// The `|` is stripped from the SQL before calling [`parse_context`].
    fn p(s: &str) -> CompletionContext {
        let pos = s
            .find('|')
            .expect("test string must contain '|' cursor marker");
        let sql: String = s.chars().filter(|&c| c != '|').collect();
        parse_context(&sql, pos)
    }

    // ── Four required cases from the issue ───────────────────────────────────

    #[test]
    fn parse_context_should_return_column_name_when_select_has_no_from_clause() {
        assert_eq!(p("SELECT |"), CompletionContext::ColumnName { table: None });
    }

    #[test]
    fn parse_context_should_return_table_name_after_from_keyword() {
        assert_eq!(p("SELECT * FROM |"), CompletionContext::TableName);
    }

    #[test]
    fn parse_context_should_return_column_name_with_table_when_cursor_is_in_select_list() {
        assert_eq!(
            p("SELECT | FROM users"),
            CompletionContext::ColumnName {
                table: Some("users".to_string())
            }
        );
    }

    #[test]
    fn parse_context_should_resolve_alias_to_table_in_dot_notation() {
        assert_eq!(
            p("SELECT u.| FROM users u"),
            CompletionContext::ColumnName {
                table: Some("users".to_string())
            }
        );
    }

    // ── Additional coverage ───────────────────────────────────────────────────

    #[test]
    fn parse_context_should_return_table_name_after_join() {
        assert_eq!(p("SELECT * FROM t JOIN |"), CompletionContext::TableName);
    }

    #[test]
    fn parse_context_should_return_table_name_after_inner_join() {
        assert_eq!(
            p("SELECT * FROM t INNER JOIN |"),
            CompletionContext::TableName
        );
    }

    #[test]
    fn parse_context_should_return_column_name_after_where() {
        assert_eq!(
            p("SELECT * FROM orders WHERE |"),
            CompletionContext::ColumnName {
                table: Some("orders".to_string())
            }
        );
    }

    #[test]
    fn parse_context_should_be_case_insensitive() {
        assert_eq!(p("select * from |"), CompletionContext::TableName);
        assert_eq!(
            p("select | from users"),
            CompletionContext::ColumnName {
                table: Some("users".to_string())
            }
        );
    }

    #[test]
    fn parse_context_should_return_none_for_empty_sql() {
        assert_eq!(parse_context("", 0), CompletionContext::None);
        assert_eq!(parse_context("   ", 0), CompletionContext::None);
    }

    #[test]
    fn parse_context_should_resolve_as_alias_in_dot_notation() {
        assert_eq!(
            p("SELECT u.| FROM users AS u"),
            CompletionContext::ColumnName {
                table: Some("users".to_string())
            }
        );
    }

    #[test]
    fn parse_context_should_return_column_name_after_order_by() {
        assert_eq!(
            p("SELECT * FROM users ORDER BY |"),
            CompletionContext::ColumnName {
                table: Some("users".to_string())
            }
        );
    }

    #[test]
    fn parse_context_should_return_column_name_after_group_by() {
        assert_eq!(
            p("SELECT * FROM users GROUP BY |"),
            CompletionContext::ColumnName {
                table: Some("users".to_string())
            }
        );
    }

    #[test]
    fn parse_context_should_return_join_condition_table_after_join_on() {
        assert_eq!(
            p("SELECT * FROM t1 JOIN t2 ON |"),
            CompletionContext::JoinConditionTable {
                tables: vec!["t1".to_string(), "t2".to_string()]
            }
        );
    }

    #[test]
    fn parse_context_should_return_next_clause_after_from_table_and_space() {
        assert_eq!(p("SELECT * FROM users |"), CompletionContext::NextClause);
    }

    #[test]
    fn parse_context_should_return_join_on_after_join_table_and_space() {
        assert_eq!(p("SELECT * FROM t1 JOIN t2 |"), CompletionContext::JoinOn);
    }

    #[test]
    fn parse_context_should_return_join_on_after_inner_join_table_and_space() {
        assert_eq!(
            p("SELECT id FROM users INNER JOIN departments |"),
            CompletionContext::JoinOn
        );
    }

    #[test]
    fn parse_context_should_return_operator_after_where_column_and_space() {
        assert_eq!(
            p("SELECT * FROM users WHERE id |"),
            CompletionContext::Operator
        );
    }

    #[test]
    fn parse_context_should_return_operator_after_on_column_and_space() {
        assert_eq!(
            p("SELECT * FROM t1 JOIN t2 ON t1.id |"),
            CompletionContext::Operator
        );
    }

    #[test]
    fn parse_context_should_return_value_expected_after_where_column_equals() {
        assert_eq!(
            p("SELECT * FROM users WHERE id = |"),
            CompletionContext::ValueExpected
        );
    }

    #[test]
    fn parse_context_should_return_value_expected_after_where_column_gt() {
        assert_eq!(
            p("SELECT * FROM users WHERE created_at > |"),
            CompletionContext::ValueExpected
        );
    }

    #[test]
    fn parse_context_should_return_none_after_semicolon() {
        assert_eq!(
            parse_context("SELECT * FROM users;", 20),
            CompletionContext::None
        );
        assert_eq!(parse_context("SELECT 1; ", 9), CompletionContext::None);
    }

    #[test]
    fn parse_context_should_return_join_condition_table_after_on_keyword() {
        let ctx = p("SELECT id FROM users INNER JOIN departments ON |");
        assert_eq!(
            ctx,
            CompletionContext::JoinConditionTable {
                tables: vec!["users".to_string(), "departments".to_string()]
            }
        );
    }

    #[test]
    fn parse_context_should_return_column_name_for_dot_notation_on_join_alias() {
        assert_eq!(
            p("SELECT * FROM t1 JOIN t2 ON t1.|"),
            CompletionContext::ColumnName {
                table: Some("t1".to_string())
            }
        );
    }

    #[test]
    fn parse_context_should_return_table_name_immediately_after_from() {
        assert_eq!(p("SELECT * FROM |"), CompletionContext::TableName);
    }

    #[test]
    fn parse_context_should_return_table_name_while_typing_after_from() {
        assert_eq!(p("SELECT * FROM use|"), CompletionContext::TableName);
    }

    // ── extract_from_table ────────────────────────────────────────────────────

    #[test]
    fn extract_from_table_should_return_first_table_name() {
        assert_eq!(
            extract_from_table("SELECT * FROM users WHERE id = 1"),
            Some("users".to_string())
        );
    }

    #[test]
    fn extract_from_table_should_return_none_when_no_from() {
        assert_eq!(extract_from_table("SELECT 1"), None);
    }

    #[test]
    fn extract_from_table_should_return_none_when_from_has_no_table() {
        assert_eq!(extract_from_table("SELECT * FROM "), None);
    }

    #[test]
    fn extract_from_table_should_be_case_insensitive() {
        assert_eq!(
            extract_from_table("select * from orders"),
            Some("orders".to_string())
        );
    }

    // ── in_select_list ────────────────────────────────────────────────────────

    #[test]
    fn in_select_list_should_return_true_between_select_and_from() {
        assert!(in_select_list("SELECT id FROM users", 9)); // after "id"
        assert!(in_select_list("SELECT ", 7)); // right after SELECT
    }

    #[test]
    fn in_select_list_should_return_false_after_from() {
        assert!(!in_select_list("SELECT id FROM users WHERE id = 1", 20)); // in WHERE
        assert!(!in_select_list("SELECT * FROM ", 14)); // in FROM
    }

    #[test]
    fn in_select_list_should_return_false_when_no_select() {
        assert!(!in_select_list("FROM users", 5));
    }

    // ── extract_referenced_tables ────────────────────────────────────────────

    #[test]
    fn extract_referenced_tables_should_return_tables_from_from_and_join() {
        let tables = extract_referenced_tables(
            "SELECT id FROM users INNER JOIN departments ON users.id = departments.id",
        );
        assert!(
            tables.contains(&"users".to_string()),
            "expected users in {tables:?}"
        );
        assert!(
            tables.contains(&"departments".to_string()),
            "expected departments in {tables:?}"
        );
    }

    #[test]
    fn extract_referenced_tables_should_deduplicate() {
        let tables =
            extract_referenced_tables("SELECT * FROM users JOIN users ON users.id = users.id");
        assert_eq!(tables.iter().filter(|t| t.as_str() == "users").count(), 1);
    }

    #[test]
    fn extract_referenced_tables_should_return_empty_when_no_from() {
        let tables = extract_referenced_tables("SELECT 1");
        assert!(tables.is_empty());
    }
}

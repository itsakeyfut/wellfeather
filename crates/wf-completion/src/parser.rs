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
        Some("FROM") | Some("JOIN") | Some("INTO") => CompletionContext::TableName,
        Some("SELECT") | Some("WHERE") | Some("SET") | Some("HAVING") => {
            match extract_from_table(sql) {
                Some(t) => CompletionContext::ColumnName { table: Some(t) },
                None => CompletionContext::Keyword,
            }
        }
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
    const TRIGGERS: &[&str] = &["SELECT", "FROM", "JOIN", "WHERE", "SET", "HAVING", "INTO"];
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
            } else if !is_join_keyword(prev) {
                // "tablename alias" pattern
                return Some(tokens_sql[i - 1].to_string());
            }
        }
    }

    // Alias not matched — fall back to first table in FROM clause.
    extract_from_table(sql)
}

/// True if `kw` is a SQL keyword that directly precedes a table name.
fn is_join_keyword(kw: &str) -> bool {
    matches!(
        kw.to_ascii_uppercase().as_str(),
        "FROM" | "JOIN" | "LEFT" | "RIGHT" | "INNER" | "OUTER" | "CROSS" | "FULL" | "NATURAL"
    )
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
    fn parse_context_should_return_keyword_when_select_has_no_from_clause() {
        assert_eq!(p("SELECT |"), CompletionContext::Keyword);
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
}

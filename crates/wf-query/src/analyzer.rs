/// Returns the SQL statement that contains `cursor_pos` (byte offset).
///
/// The input is split on `;` and the segment whose byte range covers
/// `cursor_pos` is returned, trimmed of surrounding whitespace.
/// A cursor positioned exactly on a `;` is considered part of the
/// statement that precedes it.
///
/// If the input has no semicolon the whole string is returned trimmed.
pub fn extract_statement_at(sql: &str, cursor_pos: usize) -> &str {
    let mut pos: usize = 0;
    let mut last: &str = "";
    for segment in sql.split(';') {
        let end = pos + segment.len();
        if cursor_pos <= end {
            let trimmed = segment.trim();
            if trimmed.is_empty() {
                return last;
            }
            // A cursor that lands within the leading newline whitespace of a segment
            // is visually "at end of the previous line" — attribute it to the
            // preceding statement rather than this one.
            let prefix_len = segment.len() - segment.trim_start().len();
            if cursor_pos < pos + prefix_len && segment[..prefix_len].contains('\n') {
                return last;
            }
            return trimmed;
        }
        let t = segment.trim();
        if !t.is_empty() {
            last = t;
        }
        pos = end + 1; // skip the ';'
    }
    // cursor_pos is past the end of the string
    if last.is_empty() { sql.trim() } else { last }
}

/// Splits `sql` on semicolons and returns all non-empty, trimmed statements.
///
/// Suitable for "run all" operations: each returned string is a single
/// statement ready to send to the database individually.
pub fn extract_all_statements(sql: &str) -> Vec<&str> {
    sql.split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect()
}

/// Returns `true` if `sql` contains an UPDATE or DELETE statement with no WHERE clause.
///
/// Checks all semicolon-separated statements. Intended to guard against accidental
/// full-table modifications when `safe_dml` is enabled on a connection.
pub fn has_dangerous_dml(sql: &str) -> bool {
    extract_all_statements(sql).iter().any(|stmt| {
        let upper = stmt.to_uppercase();
        (upper.starts_with("UPDATE ") || upper.starts_with("DELETE ")) && !upper.contains(" WHERE ")
    })
}

/// Returns `true` if `sql` contains any write statement (INSERT, UPDATE, DELETE, or DDL).
///
/// Checks all semicolon-separated statements. Intended to guard read-only connections
/// against accidental modifications.
pub fn is_write_statement(sql: &str) -> bool {
    const WRITE_KEYWORDS: &[&str] = &[
        "INSERT", "UPDATE", "DELETE", "CREATE", "DROP", "ALTER", "TRUNCATE",
    ];
    extract_all_statements(sql).iter().any(|stmt| {
        let upper = stmt.to_uppercase();
        WRITE_KEYWORDS.iter().any(|kw| {
            upper.starts_with(kw)
                && upper[kw.len()..].starts_with(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                || upper == *kw
        })
    })
}

/// Returns the substring of `sql` for the byte range `start..end`.
///
/// The range is clamped to valid string boundaries.  If `start > end`
/// the arguments are swapped so the function is order-independent.
pub fn extract_selection(sql: &str, start: usize, end: usize) -> &str {
    let len = sql.len();
    let a = start.min(len);
    let b = end.min(len);
    let (a, b) = if a <= b { (a, b) } else { (b, a) };
    &sql[a..b]
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── extract_statement_at ─────────────────────────────────────────────────

    #[test]
    fn extract_statement_at_should_return_whole_string_when_no_semicolon() {
        assert_eq!(extract_statement_at("SELECT 1", 0), "SELECT 1");
        assert_eq!(extract_statement_at("SELECT 1", 5), "SELECT 1");
        assert_eq!(extract_statement_at("  SELECT 1  ", 3), "SELECT 1");
    }

    #[test]
    fn extract_statement_at_should_handle_trailing_semicolon() {
        // cursor inside the statement
        assert_eq!(extract_statement_at("SELECT 1;", 0), "SELECT 1");
        assert_eq!(extract_statement_at("SELECT 1;", 7), "SELECT 1");
        // cursor ON the semicolon (byte 8) — belongs to the preceding statement
        assert_eq!(extract_statement_at("SELECT 1;", 8), "SELECT 1");
        // cursor after the semicolon — empty trailing segment returns preceding statement
        assert_eq!(extract_statement_at("SELECT 1;", 9), "SELECT 1");
    }

    #[test]
    fn extract_statement_at_should_return_correct_statement_for_multiple_statements() {
        let sql = "SELECT 1; SELECT 2; SELECT 3";
        //          0123456789012345678901234567
        //                    1111111111222222222

        // first statement ("SELECT 1", bytes 0–7)
        assert_eq!(extract_statement_at(sql, 0), "SELECT 1");
        assert_eq!(extract_statement_at(sql, 7), "SELECT 1");
        // cursor on first ';' (byte 8) → first statement
        assert_eq!(extract_statement_at(sql, 8), "SELECT 1");

        // second statement (" SELECT 2", bytes 9–17, trimmed "SELECT 2")
        assert_eq!(extract_statement_at(sql, 9), "SELECT 2");
        assert_eq!(extract_statement_at(sql, 14), "SELECT 2");
        // cursor on second ';' (byte 18) → second statement
        assert_eq!(extract_statement_at(sql, 18), "SELECT 2");

        // third statement (" SELECT 3", bytes 19–27, trimmed "SELECT 3")
        assert_eq!(extract_statement_at(sql, 19), "SELECT 3");
        assert_eq!(extract_statement_at(sql, 27), "SELECT 3");
    }

    #[test]
    fn extract_statement_at_should_trim_whitespace_around_statement() {
        let sql = "  SELECT 1  ;  SELECT 2  ";
        assert_eq!(extract_statement_at(sql, 2), "SELECT 1");
        assert_eq!(extract_statement_at(sql, 15), "SELECT 2");
    }

    #[test]
    fn extract_statement_at_should_handle_newline_separated_statements() {
        let sql = "SELECT 1;\nSELECT 2;\nSELECT 3";
        assert_eq!(extract_statement_at(sql, 0), "SELECT 1");
        assert_eq!(extract_statement_at(sql, 10), "SELECT 2");
        assert_eq!(extract_statement_at(sql, 20), "SELECT 3");
    }

    #[test]
    fn extract_statement_at_should_attribute_newline_to_preceding_statement() {
        let sql = "SELECT 1;\nSELECT 2;\nSELECT 3";
        //                    9^         19^
        // Cursor at '\n' after ';' is visually end-of-line — belongs to preceding stmt.
        assert_eq!(extract_statement_at(sql, 9), "SELECT 1");
        assert_eq!(extract_statement_at(sql, 19), "SELECT 2");
        // Cursor at first char of the next statement belongs to that statement.
        assert_eq!(extract_statement_at(sql, 10), "SELECT 2");
    }

    #[test]
    fn extract_statement_at_should_return_last_statement_when_cursor_past_end() {
        // Trailing semicolon — cursor one past the ';'
        assert_eq!(extract_statement_at("SELECT 1;", 9), "SELECT 1");
        // Trailing semicolon + newline — cursor at/past the newline
        let sql = "SELECT 1;\nSELECT 2;\n";
        assert_eq!(extract_statement_at(sql, 19), "SELECT 2");
        assert_eq!(extract_statement_at(sql, 20), "SELECT 2");
    }

    // ── has_dangerous_dml ────────────────────────────────────────────────────

    #[test]
    fn has_dangerous_dml_should_return_true_for_update_without_where() {
        assert!(has_dangerous_dml("UPDATE users SET name = 'x'"));
    }

    #[test]
    fn has_dangerous_dml_should_return_true_for_delete_without_where() {
        assert!(has_dangerous_dml("DELETE FROM orders"));
    }

    #[test]
    fn has_dangerous_dml_should_return_false_when_where_is_present() {
        assert!(!has_dangerous_dml(
            "UPDATE users SET name = 'x' WHERE id = 1"
        ));
        assert!(!has_dangerous_dml("DELETE FROM orders WHERE id = 42"));
    }

    #[test]
    fn has_dangerous_dml_should_be_case_insensitive() {
        assert!(has_dangerous_dml("update users set name = 'x'"));
        assert!(has_dangerous_dml("delete from orders"));
        assert!(!has_dangerous_dml(
            "update users set name = 'x' where id = 1"
        ));
    }

    #[test]
    fn has_dangerous_dml_should_return_false_for_select() {
        assert!(!has_dangerous_dml("SELECT * FROM users"));
    }

    #[test]
    fn has_dangerous_dml_should_return_false_for_truncate() {
        assert!(!has_dangerous_dml("TRUNCATE users"));
    }

    #[test]
    fn has_dangerous_dml_should_detect_dangerous_stmt_in_multi_statement_sql() {
        let sql = "SELECT 1; UPDATE users SET name = 'x'; SELECT 2";
        assert!(has_dangerous_dml(sql));
        let safe = "SELECT 1; UPDATE users SET name = 'x' WHERE id = 1; SELECT 2";
        assert!(!has_dangerous_dml(safe));
    }

    #[test]
    fn has_dangerous_dml_should_return_false_for_delete_from_without_where_when_keyword_is_embedded()
     {
        // "NOWHERE" or similar embedded strings must not be treated as WHERE
        assert!(has_dangerous_dml("DELETE FROM nowhere_table"));
    }

    // ── is_write_statement ───────────────────────────────────────────────────

    #[test]
    fn is_write_statement_should_return_true_for_insert() {
        assert!(is_write_statement("INSERT INTO users VALUES (1)"));
    }

    #[test]
    fn is_write_statement_should_return_true_for_update() {
        assert!(is_write_statement(
            "UPDATE users SET name = 'x' WHERE id = 1"
        ));
    }

    #[test]
    fn is_write_statement_should_return_true_for_delete() {
        assert!(is_write_statement("DELETE FROM users WHERE id = 1"));
    }

    #[test]
    fn is_write_statement_should_return_true_for_create() {
        assert!(is_write_statement("CREATE TABLE t (id INT)"));
    }

    #[test]
    fn is_write_statement_should_return_true_for_drop() {
        assert!(is_write_statement("DROP TABLE t"));
    }

    #[test]
    fn is_write_statement_should_return_true_for_alter() {
        assert!(is_write_statement("ALTER TABLE t ADD COLUMN x INT"));
    }

    #[test]
    fn is_write_statement_should_return_true_for_truncate() {
        assert!(is_write_statement("TRUNCATE users"));
    }

    #[test]
    fn is_write_statement_should_be_case_insensitive() {
        assert!(is_write_statement("insert into users values (1)"));
        assert!(is_write_statement("drop table t"));
    }

    #[test]
    fn is_write_statement_should_return_false_for_select() {
        assert!(!is_write_statement("SELECT * FROM users"));
    }

    #[test]
    fn is_write_statement_should_return_true_when_any_stmt_in_batch_is_write() {
        assert!(is_write_statement("SELECT 1; INSERT INTO users VALUES (1)"));
    }

    #[test]
    fn is_write_statement_should_not_match_keyword_as_prefix_of_identifier() {
        // "INSERTS" is not INSERT; "DROPS" is not DROP
        assert!(!is_write_statement("SELECT inserts FROM t"));
    }

    // ── extract_selection ───────────────────────────────────────────────────

    #[test]
    fn extract_selection_should_return_exact_byte_range() {
        assert_eq!(extract_selection("SELECT 1; SELECT 2", 0, 8), "SELECT 1");
        assert_eq!(extract_selection("SELECT 1; SELECT 2", 10, 18), "SELECT 2");
    }

    #[test]
    fn extract_selection_should_return_empty_for_zero_length_range() {
        assert_eq!(extract_selection("SELECT 1", 3, 3), "");
    }

    #[test]
    fn extract_selection_should_return_full_string_when_range_covers_all() {
        assert_eq!(extract_selection("SELECT 1", 0, 8), "SELECT 1");
    }

    #[test]
    fn extract_selection_should_clamp_end_to_string_length() {
        assert_eq!(extract_selection("SELECT 1", 0, 100), "SELECT 1");
    }

    #[test]
    fn extract_selection_should_swap_start_and_end_when_reversed() {
        assert_eq!(extract_selection("SELECT 1; SELECT 2", 8, 0), "SELECT 1");
    }
}

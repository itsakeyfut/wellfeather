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
    for segment in sql.split(';') {
        let end = pos + segment.len();
        if cursor_pos <= end {
            return segment.trim();
        }
        pos = end + 1; // skip the ';'
    }
    // cursor_pos is past the end of the string — return the whole text trimmed
    sql.trim()
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
        // cursor after the semicolon — empty trailing segment
        assert_eq!(extract_statement_at("SELECT 1;", 9), "");
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

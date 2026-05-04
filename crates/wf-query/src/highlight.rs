// ── Token kinds (match the Slint-side int constants) ─────────────────────────

#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Keyword = 0,
    StringLiteral = 1,
    Comment = 2,
    Number = 3,
}

// ── Output type ───────────────────────────────────────────────────────────────

/// One syntax-highlight span occupying a single rendered line.
/// `text` holds the exact characters to render (multi-line tokens are pre-split).
/// `col` is the 0-based Unicode scalar column for pixel-position computation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightSpan {
    pub line: i32,
    pub col: i32,
    pub text: String,
    pub kind: i32,
}

// ── SQL keyword table ─────────────────────────────────────────────────────────

static KEYWORDS: &[&str] = &[
    "SELECT",
    "FROM",
    "WHERE",
    "JOIN",
    "LEFT",
    "RIGHT",
    "INNER",
    "OUTER",
    "FULL",
    "CROSS",
    "ON",
    "AS",
    "AND",
    "OR",
    "NOT",
    "IN",
    "EXISTS",
    "BETWEEN",
    "LIKE",
    "ILIKE",
    "IS",
    "NULL",
    "HAVING",
    "GROUP",
    "ORDER",
    "BY",
    "LIMIT",
    "OFFSET",
    "DISTINCT",
    "UNION",
    "ALL",
    "INTERSECT",
    "EXCEPT",
    "INSERT",
    "INTO",
    "VALUES",
    "UPDATE",
    "SET",
    "DELETE",
    "CREATE",
    "TABLE",
    "VIEW",
    "INDEX",
    "DROP",
    "ALTER",
    "ADD",
    "COLUMN",
    "PRIMARY",
    "KEY",
    "FOREIGN",
    "REFERENCES",
    "CONSTRAINT",
    "UNIQUE",
    "DEFAULT",
    "CHECK",
    "RETURNING",
    "WITH",
    "RECURSIVE",
    "CASE",
    "WHEN",
    "THEN",
    "ELSE",
    "END",
    "CAST",
    "COALESCE",
    "NULLIF",
    "IF",
    "ANY",
    "SOME",
    "TRUE",
    "FALSE",
    "ASC",
    "DESC",
    "NULLS",
    "FIRST",
    "LAST",
    "PARTITION",
    "OVER",
    "WINDOW",
    "ROWS",
    "RANGE",
    "UNBOUNDED",
    "PRECEDING",
    "FOLLOWING",
    "CURRENT",
    "ROW",
    "EXPLAIN",
    "ANALYZE",
    "GRANT",
    "REVOKE",
    "TRANSACTION",
    "BEGIN",
    "COMMIT",
    "ROLLBACK",
    "SAVEPOINT",
    "TRUNCATE",
    "VACUUM",
    "PRAGMA",
    "SHOW",
    "USE",
    "DATABASE",
    "SCHEMA",
    "TRIGGER",
    "PROCEDURE",
    "FUNCTION",
    "REPLACE",
    "TEMPORARY",
    "TEMP",
    "IF",
    "NATURAL",
    "USING",
    "LATERAL",
    "APPLY",
    "PIVOT",
    "UNPIVOT",
    "MERGE",
    "MATCHED",
    "DO",
    "NOTHING",
    "CONFLICT",
    "EXCLUDE",
    "FILTER",
    "WITHIN",
    "TIES",
    "FETCH",
    "NEXT",
    "ONLY",
    "SKIP",
    "LOCKED",
    "NOWAIT",
    "SHARE",
    "MODE",
    "FOR",
    "EACH",
    "BEFORE",
    "AFTER",
    "INSTEAD",
    "OF",
    "EXECUTE",
    "CALL",
    "TYPE",
    "ENUM",
    "SEQUENCE",
    "SERIAL",
    "BIGSERIAL",
    "SMALLSERIAL",
    "AUTO_INCREMENT",
    "IDENTITY",
];

fn is_keyword(word: &str) -> bool {
    let upper: &str = &word.to_uppercase();
    KEYWORDS.contains(&upper)
}

// ── Raw token (byte-range) ────────────────────────────────────────────────────

#[derive(Debug)]
struct RawToken {
    start: usize,
    end: usize,
    kind: TokenKind,
}

// ── Tokenizer ─────────────────────────────────────────────────────────────────

/// Identifier/plain-text kind — gap spans that cover non-highlighted text.
pub const KIND_IDENTIFIER: i32 = 4;

pub fn highlight(sql: &str) -> Vec<HighlightSpan> {
    let raw = tokenize(sql);
    let spans = byte_tokens_to_spans(sql, &raw);
    fill_gaps(sql, spans)
}

/// Fill gaps between highlighted spans with identifier spans so that every
/// character on every line is covered.  The TextInput is rendered with a
/// transparent colour; all text is drawn solely through the overlay elements.
fn fill_gaps(sql: &str, mut highlighted: Vec<HighlightSpan>) -> Vec<HighlightSpan> {
    highlighted.sort_by(|a, b| a.line.cmp(&b.line).then(a.col.cmp(&b.col)));

    let lines: Vec<&str> = sql.split('\n').collect();
    let mut result = Vec::with_capacity(highlighted.len() * 2);
    let mut span_iter = highlighted.into_iter().peekable();

    for (line_idx, line_text) in lines.iter().enumerate() {
        let line_idx = line_idx as i32;
        let line_chars: Vec<char> = line_text.chars().collect();
        let line_len = line_chars.len() as i32;
        let mut cursor: i32 = 0;

        while let Some(peek) = span_iter.peek() {
            if peek.line != line_idx {
                break;
            }
            let span = span_iter.next().unwrap();
            let span_col = span.col.min(line_len);
            if span_col > cursor {
                let gap: String = line_chars[cursor as usize..span_col as usize]
                    .iter()
                    .collect();
                if !gap.is_empty() {
                    result.push(HighlightSpan {
                        line: line_idx,
                        col: cursor,
                        text: gap,
                        kind: KIND_IDENTIFIER,
                    });
                }
            }
            cursor = (span.col + span.text.chars().count() as i32).max(cursor);
            result.push(span);
        }

        if cursor < line_len {
            let tail: String = line_chars[cursor as usize..].iter().collect();
            if !tail.is_empty() {
                result.push(HighlightSpan {
                    line: line_idx,
                    col: cursor,
                    text: tail,
                    kind: KIND_IDENTIFIER,
                });
            }
        }
    }

    result
}

fn tokenize(sql: &str) -> Vec<RawToken> {
    let bytes = sql.as_bytes();
    let len = bytes.len();
    let mut tokens = Vec::new();
    let mut i = 0;

    while i < len {
        let b = bytes[i];

        // ── Line comment: -- ─────────────────────────────────────────────────
        if b == b'-' && i + 1 < len && bytes[i + 1] == b'-' {
            let start = i;
            i += 2;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            tokens.push(RawToken {
                start,
                end: i,
                kind: TokenKind::Comment,
            });
            continue;
        }

        // ── Block comment: /* ... */ ──────────────────────────────────────────
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            let start = i;
            i += 2;
            while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i += 2; // consume */
            tokens.push(RawToken {
                start,
                end: i.min(len),
                kind: TokenKind::Comment,
            });
            continue;
        }

        // ── Single-quoted string ─────────────────────────────────────────────
        if b == b'\'' {
            let start = i;
            i += 1;
            while i < len {
                if bytes[i] == b'\'' {
                    i += 1;
                    // SQL escaped quote: ''
                    if i < len && bytes[i] == b'\'' {
                        i += 1;
                        continue;
                    }
                    break;
                }
                if bytes[i] == b'\\' {
                    i += 1; // skip escaped char
                }
                i += 1;
            }
            tokens.push(RawToken {
                start,
                end: i,
                kind: TokenKind::StringLiteral,
            });
            continue;
        }

        // ── Double-quoted identifier / string ────────────────────────────────
        if b == b'"' {
            let start = i;
            i += 1;
            while i < len {
                if bytes[i] == b'"' {
                    i += 1;
                    if i < len && bytes[i] == b'"' {
                        i += 1;
                        continue;
                    }
                    break;
                }
                i += 1;
            }
            tokens.push(RawToken {
                start,
                end: i,
                kind: TokenKind::StringLiteral,
            });
            continue;
        }

        // ── Backtick-quoted identifier (MySQL) ───────────────────────────────
        if b == b'`' {
            let start = i;
            i += 1;
            while i < len && bytes[i] != b'`' {
                i += 1;
            }
            if i < len {
                i += 1;
            }
            tokens.push(RawToken {
                start,
                end: i,
                kind: TokenKind::StringLiteral,
            });
            continue;
        }

        // ── Dollar-quoted string (PostgreSQL): $$...$$ or $tag$...$tag$ ──────
        if b == b'$' {
            let tag_end = scan_dollar_tag(bytes, i);
            if let Some(tag_end) = tag_end {
                let tag = &bytes[i..tag_end];
                let start = i;
                i = tag_end;
                // Search for matching closing tag
                loop {
                    if i >= len {
                        break;
                    }
                    if bytes[i] == b'$' {
                        // Check if closing tag matches
                        if bytes[i..].starts_with(tag) {
                            i += tag.len();
                            break;
                        }
                    }
                    i += 1;
                }
                tokens.push(RawToken {
                    start,
                    end: i,
                    kind: TokenKind::StringLiteral,
                });
                continue;
            }
        }

        // ── Numeric literal ──────────────────────────────────────────────────
        if b.is_ascii_digit() || (b == b'.' && i + 1 < len && bytes[i + 1].is_ascii_digit()) {
            let start = i;
            // Integer or decimal part
            while i < len && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i < len && bytes[i] == b'.' {
                i += 1;
                while i < len && bytes[i].is_ascii_digit() {
                    i += 1;
                }
            }
            // Exponent
            if i < len && (bytes[i] == b'e' || bytes[i] == b'E') {
                i += 1;
                if i < len && (bytes[i] == b'+' || bytes[i] == b'-') {
                    i += 1;
                }
                while i < len && bytes[i].is_ascii_digit() {
                    i += 1;
                }
            }
            // Hex literal: 0x...
            if i == start + 1
                && bytes[start] == b'0'
                && i < len
                && (bytes[i] == b'x' || bytes[i] == b'X')
            {
                i += 1;
                while i < len && bytes[i].is_ascii_hexdigit() {
                    i += 1;
                }
            }
            tokens.push(RawToken {
                start,
                end: i,
                kind: TokenKind::Number,
            });
            continue;
        }

        // ── Identifier or keyword ────────────────────────────────────────────
        if b.is_ascii_alphabetic() || b == b'_' {
            let start = i;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &sql[start..i];
            if is_keyword(word) {
                tokens.push(RawToken {
                    start,
                    end: i,
                    kind: TokenKind::Keyword,
                });
            }
            continue;
        }

        i += 1;
    }

    tokens
}

/// Scan a dollar-quote opening tag starting at `pos` (the first `$`).
/// Returns Some(end) where `bytes[pos..end]` is the full opening tag (e.g. `$$` or `$tag$`).
fn scan_dollar_tag(bytes: &[u8], pos: usize) -> Option<usize> {
    let mut i = pos + 1;
    let len = bytes.len();
    // Tag body: letters, digits, underscore
    while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
        i += 1;
    }
    if i < len && bytes[i] == b'$' {
        Some(i + 1)
    } else {
        None
    }
}

// ── Convert byte-range tokens to line/col/text spans ─────────────────────────

fn byte_tokens_to_spans(sql: &str, tokens: &[RawToken]) -> Vec<HighlightSpan> {
    if tokens.is_empty() {
        return Vec::new();
    }

    let mut spans = Vec::with_capacity(tokens.len());
    let line_starts = build_line_starts(sql);

    for tok in tokens {
        emit_spans(sql, &line_starts, tok, &mut spans);
    }

    spans
}

/// Returns a vec where `line_starts[i]` is the byte offset of the start of line `i`.
fn build_line_starts(sql: &str) -> Vec<usize> {
    let mut starts = vec![0usize];
    for (i, b) in sql.bytes().enumerate() {
        if b == b'\n' {
            starts.push(i + 1);
        }
    }
    starts
}

/// Given a token's byte range, emit one HighlightSpan per line it spans.
/// Each span carries the exact substring text for that line segment.
fn emit_spans(sql: &str, line_starts: &[usize], tok: &RawToken, out: &mut Vec<HighlightSpan>) {
    let start_byte = tok.start;
    let end_byte = tok.end.min(sql.len());
    if start_byte >= end_byte {
        return;
    }

    let start_line = find_line(line_starts, start_byte);
    let end_line = find_line(line_starts, end_byte.saturating_sub(1));

    if start_line == end_line {
        // Single-line span
        let line_start_byte = line_starts[start_line];
        let col = char_count_between(sql, line_start_byte, start_byte);
        let text = sql[start_byte..end_byte].to_string();
        if !text.is_empty() {
            out.push(HighlightSpan {
                line: start_line as i32,
                col: col as i32,
                text,
                kind: tok.kind as i32,
            });
        }
    } else {
        // Multi-line: emit one span per line

        // First line: from token start to end of that line (before \n)
        {
            let line_start_byte = line_starts[start_line];
            let col = char_count_between(sql, line_start_byte, start_byte);
            let line_end_byte = if start_line + 1 < line_starts.len() {
                line_starts[start_line + 1].saturating_sub(1)
            } else {
                sql.len()
            };
            let text = sql[start_byte..line_end_byte.min(sql.len())].to_string();
            if !text.is_empty() {
                out.push(HighlightSpan {
                    line: start_line as i32,
                    col: col as i32,
                    text,
                    kind: tok.kind as i32,
                });
            }
        }
        // Middle lines: full line content (before \n)
        for line in (start_line + 1)..end_line {
            let line_start_byte = line_starts[line];
            let line_end_byte = if line + 1 < line_starts.len() {
                line_starts[line + 1].saturating_sub(1)
            } else {
                sql.len()
            };
            let text = sql[line_start_byte..line_end_byte.min(sql.len())].to_string();
            if !text.is_empty() {
                out.push(HighlightSpan {
                    line: line as i32,
                    col: 0,
                    text,
                    kind: tok.kind as i32,
                });
            }
        }
        // Last line: from line start to end_byte
        {
            let line_start_byte = line_starts[end_line];
            let text = sql[line_start_byte..end_byte.min(sql.len())].to_string();
            if !text.is_empty() {
                out.push(HighlightSpan {
                    line: end_line as i32,
                    col: 0,
                    text,
                    kind: tok.kind as i32,
                });
            }
        }
    }
}

/// Find the line index for a given byte offset using binary search.
fn find_line(line_starts: &[usize], byte_offset: usize) -> usize {
    match line_starts.binary_search(&byte_offset) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),
    }
}

/// Count Unicode scalar values in `sql[start..end]`.
fn char_count_between(sql: &str, start: usize, end: usize) -> usize {
    if start >= end || start >= sql.len() {
        return 0;
    }
    let end = end.min(sql.len());
    sql[start..end].chars().count()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(sql: &str) -> Vec<HighlightSpan> {
        highlight(sql)
    }

    #[test]
    fn highlight_should_detect_select_keyword() {
        let result = spans("SELECT 1");
        assert!(
            result
                .iter()
                .any(|s| s.text == "SELECT" && s.kind == TokenKind::Keyword as i32)
        );
    }

    #[test]
    fn highlight_should_preserve_original_casing_for_keywords() {
        let result = spans("select 1");
        assert!(
            result
                .iter()
                .any(|s| s.text == "select" && s.kind == TokenKind::Keyword as i32)
        );
    }

    #[test]
    fn highlight_should_detect_number_literal() {
        let result = spans("SELECT 42");
        let num = result.iter().find(|s| s.kind == TokenKind::Number as i32);
        assert!(num.is_some());
        let num = num.unwrap();
        assert_eq!(num.text, "42");
        assert_eq!(num.col, 7);
    }

    #[test]
    fn highlight_should_detect_single_quoted_string() {
        let result = spans("SELECT 'hello'");
        let s = result
            .iter()
            .find(|s| s.kind == TokenKind::StringLiteral as i32);
        assert!(s.is_some());
        assert_eq!(s.unwrap().text, "'hello'");
    }

    #[test]
    fn highlight_should_detect_line_comment() {
        let result = spans("SELECT 1 -- comment");
        let c = result.iter().find(|s| s.kind == TokenKind::Comment as i32);
        assert!(c.is_some());
        let c = c.unwrap();
        assert_eq!(c.line, 0);
        assert_eq!(c.col, 9);
        assert!(c.text.starts_with("--"));
    }

    #[test]
    fn highlight_should_detect_block_comment() {
        let result = spans("/* hello */");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].kind, TokenKind::Comment as i32);
        assert_eq!(result[0].text, "/* hello */");
    }

    #[test]
    fn highlight_should_split_multiline_block_comment() {
        let result = spans("/* line1\nline2 */");
        let comments: Vec<_> = result
            .iter()
            .filter(|s| s.kind == TokenKind::Comment as i32)
            .collect();
        assert_eq!(comments.len(), 2);
        assert_eq!(comments[0].line, 0);
        assert_eq!(comments[1].line, 1);
    }

    #[test]
    fn highlight_should_be_case_insensitive_for_keyword_detection() {
        let lower = spans("select 1");
        let upper = spans("SELECT 1");
        assert!(lower.iter().any(|s| s.kind == TokenKind::Keyword as i32));
        assert!(upper.iter().any(|s| s.kind == TokenKind::Keyword as i32));
    }

    #[test]
    fn highlight_should_not_highlight_non_keywords() {
        let result = spans("mycolumn");
        assert!(result.iter().all(|s| s.kind == KIND_IDENTIFIER));
    }

    #[test]
    fn highlight_should_handle_multiline_sql() {
        let sql = "SELECT id\nFROM users\nWHERE id = 1";
        let result = spans(sql);
        let keywords: Vec<_> = result
            .iter()
            .filter(|s| s.kind == TokenKind::Keyword as i32)
            .collect();
        assert!(keywords.len() >= 3);
        assert!(keywords.iter().any(|s| s.line == 0 && s.col == 0));
        assert!(keywords.iter().any(|s| s.line == 1 && s.col == 0));
        assert!(keywords.iter().any(|s| s.line == 2 && s.col == 0));
    }

    #[test]
    fn highlight_should_handle_empty_input() {
        assert!(highlight("").is_empty());
    }

    #[test]
    fn highlight_should_handle_double_quoted_string() {
        let result = spans(r#"SELECT "name" FROM t"#);
        assert!(
            result
                .iter()
                .any(|s| s.kind == TokenKind::StringLiteral as i32)
        );
    }

    #[test]
    fn highlight_should_detect_float_number() {
        let result = spans("SELECT 3.14");
        let num = result.iter().find(|s| s.kind == TokenKind::Number as i32);
        assert!(num.is_some());
        assert_eq!(num.unwrap().text, "3.14");
    }

    #[test]
    fn highlight_should_not_highlight_identifier_with_keyword_prefix() {
        let result = spans("selector");
        assert!(result.iter().all(|s| s.kind != TokenKind::Keyword as i32));
    }

    #[test]
    fn highlight_should_detect_escaped_single_quote_in_string() {
        let result = spans("SELECT 'it''s'");
        let s = result
            .iter()
            .find(|s| s.kind == TokenKind::StringLiteral as i32);
        assert!(s.is_some());
        assert_eq!(s.unwrap().text, "'it''s'");
    }
}

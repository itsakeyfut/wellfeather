use std::collections::{HashMap, HashSet};

/// Extract all distinct `:name` parameter placeholders from `sql`, in first-appearance order.
///
/// Colons inside string literals (single-quoted, double-quoted, backtick, dollar-quoted) and
/// inside comments (`--` line comments, `/* */` block comments) are ignored.
pub fn extract_params(sql: &str) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut result: Vec<String> = Vec::new();

    let bytes = sql.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        let b = bytes[i];

        // ── Line comment ──────────────────────────────────────────────────────
        if b == b'-' && i + 1 < len && bytes[i + 1] == b'-' {
            i += 2;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        // ── Block comment ─────────────────────────────────────────────────────
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
            continue;
        }

        // ── Single-quoted string ──────────────────────────────────────────────
        if b == b'\'' {
            i += 1;
            while i < len {
                if bytes[i] == b'\'' {
                    i += 1;
                    if i < len && bytes[i] == b'\'' {
                        i += 1;
                        continue;
                    }
                    break;
                }
                if bytes[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            continue;
        }

        // ── Double-quoted identifier / string ─────────────────────────────────
        if b == b'"' {
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
            continue;
        }

        // ── Backtick-quoted identifier (MySQL) ────────────────────────────────
        if b == b'`' {
            i += 1;
            while i < len && bytes[i] != b'`' {
                i += 1;
            }
            if i < len {
                i += 1;
            }
            continue;
        }

        // ── Dollar-quoted string (PostgreSQL) ─────────────────────────────────
        if b == b'$'
            && let Some(tag_end) = scan_dollar_tag(bytes, i)
        {
            let tag = &bytes[i..tag_end];
            i = tag_end;
            loop {
                if i >= len {
                    break;
                }
                if bytes[i] == b'$' && bytes[i..].starts_with(tag) {
                    i += tag.len();
                    break;
                }
                i += 1;
            }
            continue;
        }

        // ── Parameter placeholder :name ───────────────────────────────────────
        // Exclude :: (PostgreSQL cast operator): if previous byte is also ':', skip.
        if b == b':'
            && i + 1 < len
            && (bytes[i + 1].is_ascii_alphabetic() || bytes[i + 1] == b'_')
            && (i == 0 || bytes[i - 1] != b':')
        {
            i += 1;
            let start = i;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let name = sql[start..i].to_string();
            if seen.insert(name.clone()) {
                result.push(name);
            }
            continue;
        }

        i += 1;
    }

    result
}

/// Replace every `:name` placeholder in `sql` with the corresponding value from `values`.
///
/// Placeholders inside string literals and comments are skipped, matching the same rules
/// as [`extract_params`]. Replacements are applied back-to-front to preserve byte offsets.
pub fn substitute_params(sql: &str, values: &HashMap<String, String>) -> String {
    let mut replacements: Vec<(usize, usize, &str)> = Vec::new();

    let bytes = sql.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        let b = bytes[i];

        if b == b'-' && i + 1 < len && bytes[i + 1] == b'-' {
            i += 2;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
            continue;
        }

        if b == b'\'' {
            i += 1;
            while i < len {
                if bytes[i] == b'\'' {
                    i += 1;
                    if i < len && bytes[i] == b'\'' {
                        i += 1;
                        continue;
                    }
                    break;
                }
                if bytes[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            continue;
        }

        if b == b'"' {
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
            continue;
        }

        if b == b'`' {
            i += 1;
            while i < len && bytes[i] != b'`' {
                i += 1;
            }
            if i < len {
                i += 1;
            }
            continue;
        }

        if b == b'$'
            && let Some(tag_end) = scan_dollar_tag(bytes, i)
        {
            let tag = &bytes[i..tag_end];
            i = tag_end;
            loop {
                if i >= len {
                    break;
                }
                if bytes[i] == b'$' && bytes[i..].starts_with(tag) {
                    i += tag.len();
                    break;
                }
                i += 1;
            }
            continue;
        }

        if b == b':'
            && i + 1 < len
            && (bytes[i + 1].is_ascii_alphabetic() || bytes[i + 1] == b'_')
            && (i == 0 || bytes[i - 1] != b':')
        {
            let colon_pos = i;
            i += 1;
            let start = i;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let name = &sql[start..i];
            if let Some(replacement) = values.get(name) {
                replacements.push((colon_pos, i, replacement.as_str()));
            }
            continue;
        }

        i += 1;
    }

    // Apply replacements from end to start to preserve earlier offsets.
    let mut result = sql.to_string();
    for (start, end, replacement) in replacements.into_iter().rev() {
        result.replace_range(start..end, replacement);
    }
    result
}

fn scan_dollar_tag(bytes: &[u8], pos: usize) -> Option<usize> {
    let mut i = pos + 1;
    let len = bytes.len();
    while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
        i += 1;
    }
    if i < len && bytes[i] == b'$' {
        Some(i + 1)
    } else {
        None
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_params_should_find_basic_placeholders() {
        let params = extract_params("SELECT :id, :name FROM users");
        assert_eq!(params, vec!["id", "name"]);
    }

    #[test]
    fn extract_params_should_ignore_colons_in_single_quoted_strings() {
        let params = extract_params("SELECT ':name' FROM t");
        assert!(params.is_empty());
    }

    #[test]
    fn extract_params_should_ignore_colons_in_line_comments() {
        let params = extract_params("SELECT 1 -- :name\nFROM t");
        assert!(params.is_empty());
    }

    #[test]
    fn extract_params_should_ignore_colons_in_block_comments() {
        let params = extract_params("SELECT /* :name */ 1");
        assert!(params.is_empty());
    }

    #[test]
    fn extract_params_should_deduplicate_param_names() {
        let params = extract_params("WHERE a = :id AND b = :id");
        assert_eq!(params, vec!["id"]);
    }

    #[test]
    fn extract_params_should_handle_empty_sql() {
        assert!(extract_params("").is_empty());
    }

    #[test]
    fn extract_params_should_preserve_first_occurrence_order() {
        let params = extract_params("SELECT :b, :a, :b");
        assert_eq!(params, vec!["b", "a"]);
    }

    #[test]
    fn extract_params_should_ignore_bare_colon_not_followed_by_ident() {
        let params = extract_params("a::text");
        assert!(params.is_empty());
    }

    #[test]
    fn extract_params_should_ignore_colons_in_dollar_quoted_strings() {
        let params = extract_params("$$ :name $$");
        assert!(params.is_empty());
    }

    #[test]
    fn substitute_params_should_replace_named_placeholder() {
        let mut values = HashMap::new();
        values.insert("id".to_string(), "42".to_string());
        let result = substitute_params("SELECT :id", &values);
        assert_eq!(result, "SELECT 42");
    }

    #[test]
    fn substitute_params_should_replace_multiple_placeholders() {
        let mut values = HashMap::new();
        values.insert("id".to_string(), "1".to_string());
        values.insert("name".to_string(), "'alice'".to_string());
        let result = substitute_params("WHERE id = :id AND name = :name", &values);
        assert_eq!(result, "WHERE id = 1 AND name = 'alice'");
    }

    #[test]
    fn substitute_params_should_not_replace_inside_string_literals() {
        let mut values = HashMap::new();
        values.insert("name".to_string(), "replaced".to_string());
        let result = substitute_params("SELECT ':name'", &values);
        assert_eq!(result, "SELECT ':name'");
    }

    #[test]
    fn substitute_params_should_replace_duplicate_occurrences() {
        let mut values = HashMap::new();
        values.insert("id".to_string(), "7".to_string());
        let result = substitute_params("WHERE a = :id OR b = :id", &values);
        assert_eq!(result, "WHERE a = 7 OR b = 7");
    }
}

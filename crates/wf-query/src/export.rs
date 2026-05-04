use std::path::Path;

const UTF8_BOM: &[u8] = b"\xef\xbb\xbf";

/// Serialise `columns` + `rows` to UTF-8 BOM CSV bytes.
/// NULL cells become empty strings.  Used by [`export_csv`] and in unit tests.
pub fn result_to_csv_bytes(columns: &[String], rows: &[Vec<Option<String>>]) -> Vec<u8> {
    let mut buf: Vec<u8> = Vec::new();
    buf.extend_from_slice(UTF8_BOM);
    {
        let mut wtr = csv::Writer::from_writer(&mut buf);
        // SAFETY: writing to Vec<u8> is infallible; the only error path in csv::Writer is I/O failure, which cannot occur for an in-memory buffer.
        wtr.write_record(columns).unwrap();
        for row in rows {
            let cells: Vec<&str> = row.iter().map(|c| c.as_deref().unwrap_or("")).collect();
            // SAFETY: same as above — writing to Vec<u8> is infallible.
            wtr.write_record(&cells).unwrap();
        }
        // SAFETY: flushing a Vec<u8>-backed writer is infallible.
        wtr.flush().unwrap();
    }
    buf
}

/// Serialise `columns` + `rows` to JSON bytes (pretty-printed array of objects).
///
/// Type coercion rules applied to each cell value:
/// - `None`                → JSON `null`
/// - Parseable as `i64`   → JSON integer
/// - Parseable as `f64`   → JSON float (non-finite values fall back to string)
/// - Otherwise            → JSON string
///
/// Column order is preserved (requires the `preserve_order` feature of `serde_json`).
pub fn result_to_json_bytes(columns: &[String], rows: &[Vec<Option<String>>]) -> Vec<u8> {
    let array: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let obj: serde_json::Map<String, serde_json::Value> = columns
                .iter()
                .zip(row.iter())
                .map(|(col, cell)| {
                    let val = match cell {
                        None => serde_json::Value::Null,
                        Some(s) => {
                            if let Ok(n) = s.parse::<i64>() {
                                serde_json::Value::Number(n.into())
                            } else if let Ok(f) = s.parse::<f64>() {
                                // from_f64 returns None for NaN / ±Infinity
                                serde_json::Number::from_f64(f)
                                    .map(serde_json::Value::Number)
                                    .unwrap_or_else(|| serde_json::Value::String(s.clone()))
                            } else {
                                serde_json::Value::String(s.clone())
                            }
                        }
                    };
                    (col.clone(), val)
                })
                .collect();
            serde_json::Value::Object(obj)
        })
        .collect();
    // SAFETY: serializing to Vec<u8> is infallible; the only error path is OOM,
    // which the Rust allocator handles as a process abort before returning Err.
    serde_json::to_vec_pretty(&serde_json::Value::Array(array)).unwrap()
}

/// Write a JSON file at `path`.  See [`result_to_json_bytes`] for format details.
pub fn export_json(
    columns: &[String],
    rows: &[Vec<Option<String>>],
    path: &Path,
) -> anyhow::Result<()> {
    let bytes = result_to_json_bytes(columns, rows);
    std::fs::write(path, bytes)?;
    Ok(())
}

/// Serialise `columns` + `rows` to a batch INSERT SQL string.
///
/// Output format: one `INSERT INTO "<table>" (<cols>) VALUES (<vals>);` per row.
/// - `None` cells become the SQL `NULL` literal.
/// - String values are single-quote escaped (`'` → `''`).
/// - Column and table names are double-quoted with `"` escaped as `""`.
pub fn result_to_insert_sql(
    columns: &[String],
    rows: &[Vec<Option<String>>],
    table_name: &str,
) -> String {
    if rows.is_empty() || columns.is_empty() {
        return String::new();
    }
    let quoted_table = format!("\"{}\"", table_name.replace('"', "\"\""));
    let col_list: String = columns
        .iter()
        .map(|c| format!("\"{}\"", c.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(", ");

    let mut buf = String::new();
    for row in rows {
        let values: String = row
            .iter()
            .map(|cell| match cell {
                None => "NULL".to_string(),
                Some(s) => format!("'{}'", s.replace('\'', "''")),
            })
            .collect::<Vec<_>>()
            .join(", ");
        buf.push_str(&format!(
            "INSERT INTO {quoted_table} ({col_list}) VALUES ({values});\n"
        ));
    }
    buf
}

/// Write an INSERT SQL file at `path`.  See [`result_to_insert_sql`] for format details.
pub fn export_insert_sql(
    columns: &[String],
    rows: &[Vec<Option<String>>],
    table_name: &str,
    path: &Path,
) -> anyhow::Result<()> {
    let content = result_to_insert_sql(columns, rows, table_name);
    std::fs::write(path, content.as_bytes())?;
    Ok(())
}

/// Write a UTF-8 BOM CSV file at `path`.  NULL cells become empty strings.
pub fn export_csv(
    columns: &[String],
    rows: &[Vec<Option<String>>],
    path: &Path,
) -> anyhow::Result<()> {
    let bytes = result_to_csv_bytes(columns, rows);
    std::fs::write(path, bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── result_to_insert_sql ──────────────────────────────────────────────────

    #[test]
    fn result_to_insert_sql_should_return_empty_for_no_rows() {
        let cols = vec!["id".to_string()];
        let rows: Vec<Vec<Option<String>>> = vec![];
        assert!(result_to_insert_sql(&cols, &rows, "t").is_empty());
    }

    #[test]
    fn result_to_insert_sql_should_generate_one_insert_per_row() {
        let cols = vec!["id".to_string(), "name".to_string()];
        let rows = vec![
            vec![Some("1".to_string()), Some("Alice".to_string())],
            vec![Some("2".to_string()), Some("Bob".to_string())],
        ];
        let sql = result_to_insert_sql(&cols, &rows, "users");
        let lines: Vec<&str> = sql.trim().lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("INSERT INTO \"users\""));
        assert!(lines[0].contains("\"id\""));
        assert!(lines[0].contains("'Alice'"));
        assert!(lines[1].contains("'Bob'"));
    }

    #[test]
    fn result_to_insert_sql_should_write_null_literal_for_none_cells() {
        let cols = vec!["a".to_string(), "b".to_string()];
        let rows = vec![vec![Some("hello".to_string()), None]];
        let sql = result_to_insert_sql(&cols, &rows, "t");
        assert!(sql.contains("NULL"), "expected NULL literal: {sql}");
        assert!(!sql.contains("''"), "NULL must not be empty string: {sql}");
    }

    #[test]
    fn result_to_insert_sql_should_escape_single_quotes_in_values() {
        let cols = vec!["v".to_string()];
        let rows = vec![vec![Some("it's a test".to_string())]];
        let sql = result_to_insert_sql(&cols, &rows, "t");
        assert!(
            sql.contains("'it''s a test'"),
            "expected escaped quote: {sql}"
        );
    }

    #[test]
    fn result_to_insert_sql_should_quote_table_and_column_names() {
        let cols = vec!["my col".to_string()];
        let rows = vec![vec![Some("v".to_string())]];
        let sql = result_to_insert_sql(&cols, &rows, "my table");
        assert!(sql.contains("\"my table\""), "expected quoted table: {sql}");
        assert!(sql.contains("\"my col\""), "expected quoted column: {sql}");
    }

    #[test]
    fn result_to_insert_sql_should_escape_double_quotes_in_identifiers() {
        let cols = vec!["a\"b".to_string()];
        let rows = vec![vec![Some("v".to_string())]];
        let sql = result_to_insert_sql(&cols, &rows, "t\"t");
        assert!(
            sql.contains("\"t\"\"t\""),
            "expected escaped table name: {sql}"
        );
        assert!(
            sql.contains("\"a\"\"b\""),
            "expected escaped column name: {sql}"
        );
    }

    // ── result_to_json_bytes ──────────────────────────────────────────────────

    #[test]
    fn result_to_json_bytes_should_map_null_to_json_null() {
        let cols = vec!["a".to_string(), "b".to_string()];
        let rows = vec![vec![Some("hello".to_string()), None]];
        let bytes = result_to_json_bytes(&cols, &rows);
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            parsed[0]["a"],
            serde_json::Value::String("hello".to_string())
        );
        assert_eq!(parsed[0]["b"], serde_json::Value::Null);
    }

    #[test]
    fn result_to_json_bytes_should_coerce_numeric_strings_to_json_numbers() {
        let cols = vec!["i".to_string(), "f".to_string(), "s".to_string()];
        let rows = vec![vec![
            Some("42".to_string()),
            Some("1.5".to_string()),
            Some("hello".to_string()),
        ]];
        let bytes = result_to_json_bytes(&cols, &rows);
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed[0]["i"], serde_json::json!(42));
        assert_eq!(parsed[0]["f"], serde_json::json!(1.5));
        assert_eq!(parsed[0]["s"], serde_json::json!("hello"));
    }

    #[test]
    fn result_to_csv_bytes_should_include_bom_and_header() {
        let cols = vec!["id".to_string(), "name".to_string()];
        let rows: Vec<Vec<Option<String>>> = vec![];
        let bytes = result_to_csv_bytes(&cols, &rows);
        assert!(bytes.starts_with(b"\xef\xbb\xbf"), "BOM missing");
        let text = std::str::from_utf8(&bytes[3..]).unwrap();
        assert!(text.contains("id"), "header missing");
        assert!(text.contains("name"), "header missing");
    }

    #[test]
    fn result_to_csv_bytes_should_export_null_as_empty_string() {
        let cols = vec!["a".to_string(), "b".to_string()];
        let rows = vec![vec![Some("hello".to_string()), None]];
        let bytes = result_to_csv_bytes(&cols, &rows);
        let text = std::str::from_utf8(&bytes[3..]).unwrap();
        assert!(text.contains("hello"), "value missing");
        assert!(text.contains("hello,"), "NULL not serialised as empty");
    }

    #[test]
    fn result_to_csv_bytes_should_escape_commas_in_values() {
        let cols = vec!["v".to_string()];
        let rows = vec![vec![Some("a,b".to_string())]];
        let bytes = result_to_csv_bytes(&cols, &rows);
        let text = std::str::from_utf8(&bytes[3..]).unwrap();
        assert!(text.contains("\"a,b\""), "comma not escaped: {text}");
    }
}

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

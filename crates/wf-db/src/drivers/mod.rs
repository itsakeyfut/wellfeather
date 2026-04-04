pub mod my;
pub mod pg;
pub mod sqlite;

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Returns `true` if `sql` is expected to return rows (SELECT, WITH, PRAGMA,
/// SHOW, EXPLAIN, DESCRIBE, VALUES, TABLE), and `false` for DML / DDL
/// statements (INSERT, UPDATE, DELETE, CREATE, DROP, ALTER, TRUNCATE, …).
///
/// Only the leading keyword is inspected, so the check is intentionally
/// simple. Edge-cases like a CTE starting with `WITH` are handled correctly
/// because `WITH … SELECT` is row-returning by its first keyword.
pub(super) fn is_row_returning(sql: &str) -> bool {
    let keyword: String = sql
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect();
    matches!(
        keyword.to_ascii_uppercase().as_str(),
        "SELECT"
            | "WITH"
            | "PRAGMA"
            | "SHOW"
            | "EXPLAIN"
            | "DESCRIBE"
            | "DESC"
            | "VALUES"
            | "TABLE"
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_row_returning_should_return_true_for_select() {
        assert!(is_row_returning("SELECT 1"));
        assert!(is_row_returning("  select * from t"));
        assert!(is_row_returning("WITH cte AS (SELECT 1) SELECT * FROM cte"));
    }

    #[test]
    fn is_row_returning_should_return_false_for_dml() {
        assert!(!is_row_returning("INSERT INTO t VALUES (1)"));
        assert!(!is_row_returning("UPDATE t SET x = 1"));
        assert!(!is_row_returning("DELETE FROM t"));
        assert!(!is_row_returning("CREATE TABLE t (id INTEGER)"));
        assert!(!is_row_returning("DROP TABLE t"));
        assert!(!is_row_returning("ALTER TABLE t ADD COLUMN x TEXT"));
    }
}

use std::time::Instant;

use sqlx::{Column, MySqlPool, Row, TypeInfo};

use crate::error::DbError;
use crate::models::QueryResult;

/// Connect to a MySQL database at `url`.
///
/// Any sqlx connection error is wrapped as [`DbError::ConnectionFailed`].
pub async fn connect(url: &str) -> Result<MySqlPool, DbError> {
    MySqlPool::connect(url)
        .await
        .map_err(|e| DbError::ConnectionFailed(e.to_string()))
}

/// Execute `sql` against `pool` and return a [`QueryResult`].
///
/// - **SELECT / row-returning statements**: columns and rows are populated.
/// - **DML / DDL statements**: rows are empty; `row_count` = `rows_affected()`.
/// - **NULL values** map to `None`.
/// - `execution_time_ms` is measured with [`Instant`].
pub async fn execute(pool: &MySqlPool, sql: &str) -> Result<QueryResult, DbError> {
    let started = Instant::now();

    if super::is_row_returning(sql) {
        let rows = sqlx::query(sql)
            .fetch_all(pool)
            .await
            .map_err(DbError::from)?;

        let columns: Vec<String> = rows
            .first()
            .map(|r| r.columns().iter().map(|c| c.name().to_string()).collect())
            .unwrap_or_default();

        let data: Vec<Vec<Option<String>>> = rows
            .iter()
            .map(|row| (0..row.len()).map(|i| cell_to_string(row, i)).collect())
            .collect();

        let row_count = data.len();
        Ok(QueryResult {
            columns,
            rows: data,
            row_count,
            execution_time_ms: started.elapsed().as_millis(),
        })
    } else {
        let result = sqlx::query(sql)
            .execute(pool)
            .await
            .map_err(DbError::from)?;

        Ok(QueryResult {
            columns: vec![],
            rows: vec![],
            row_count: result.rows_affected() as usize,
            execution_time_ms: started.elapsed().as_millis(),
        })
    }
}

// ---------------------------------------------------------------------------
// Cell decoding
// ---------------------------------------------------------------------------

/// Convert a single MySQL cell to `Option<String>`.
///
/// NULL values are detected first (via `Option<T>` decode returning `Ok(None)`).
/// Each column type is then matched against common MySQL type names and decoded
/// with an appropriate Rust type before stringification.
fn cell_to_string(row: &sqlx::mysql::MySqlRow, i: usize) -> Option<String> {
    // Step 1 — NULL + text-like types (VARCHAR, CHAR, TEXT, ENUM, SET, …)
    if let Ok(v) = row.try_get::<Option<String>, _>(i) {
        return v;
    }

    // Step 2 — type-specific decode
    let col_type = row.column(i).type_info().name().to_ascii_uppercase();
    let col_type = col_type.as_str();

    // Integer types (MySQL type names are uppercased by sqlx)
    if matches!(
        col_type,
        "TINYINT"
            | "SMALLINT"
            | "MEDIUMINT"
            | "INT"
            | "BIGINT"
            | "TINYINT UNSIGNED"
            | "SMALLINT UNSIGNED"
            | "MEDIUMINT UNSIGNED"
            | "INT UNSIGNED"
            | "BIGINT UNSIGNED"
    ) {
        return row
            .try_get::<Option<i64>, _>(i)
            .ok()
            .flatten()
            .map(|v| v.to_string());
    }

    // Floating-point and decimal types
    if matches!(col_type, "FLOAT" | "DOUBLE" | "DECIMAL" | "NUMERIC") {
        return row
            .try_get::<Option<f64>, _>(i)
            .ok()
            .flatten()
            .map(|v| v.to_string());
    }

    // Boolean (stored as TINYINT(1) in MySQL; type_info may still say TINYINT)
    if col_type == "BOOLEAN" || col_type == "BOOL" {
        return row
            .try_get::<Option<bool>, _>(i)
            .ok()
            .flatten()
            .map(|v| v.to_string());
    }

    // Date / time (requires sqlx "chrono" feature)
    if col_type == "DATETIME" || col_type == "TIMESTAMP" {
        return row
            .try_get::<Option<chrono::NaiveDateTime>, _>(i)
            .ok()
            .flatten()
            .map(|v| v.to_string());
    }
    if col_type == "DATE" {
        return row
            .try_get::<Option<chrono::NaiveDate>, _>(i)
            .ok()
            .flatten()
            .map(|v| v.to_string());
    }
    if col_type == "TIME" {
        return row
            .try_get::<Option<chrono::NaiveTime>, _>(i)
            .ok()
            .flatten()
            .map(|v| v.to_string());
    }

    // Binary types
    if matches!(
        col_type,
        "BLOB" | "MEDIUMBLOB" | "LONGBLOB" | "TINYBLOB" | "BINARY" | "VARBINARY"
    ) {
        return Some("<BLOB>".to_string());
    }

    // Fallback — cascade through numeric types
    row.try_get::<Option<i64>, _>(i)
        .ok()
        .flatten()
        .map(|v| v.to_string())
        .or_else(|| {
            row.try_get::<Option<f64>, _>(i)
                .ok()
                .flatten()
                .map(|v| v.to_string())
        })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Requires a running MySQL instance.
    /// Set `TEST_MY_URL` to override the default connection string.
    /// Run with: `cargo test -p wf-db -- --ignored`
    #[tokio::test]
    #[ignore]
    async fn connect_should_succeed_with_real_mysql() {
        let url = std::env::var("TEST_MY_URL")
            .unwrap_or_else(|_| "mysql://root:root@localhost:3306/mysql".to_string());
        let pool = connect(&url).await.unwrap();
        let row: (i32,) = sqlx::query_as("SELECT 1").fetch_one(&pool).await.unwrap();
        assert_eq!(row.0, 1);
    }

    #[tokio::test]
    #[ignore]
    async fn connect_should_return_connection_failed_on_unreachable_host() {
        let result = connect("mysql://user:pass@127.0.0.1:19999/db").await;
        assert!(matches!(result, Err(DbError::ConnectionFailed(_))));
    }

    #[tokio::test]
    #[ignore]
    async fn execute_select_should_return_rows_with_null_values() {
        let url = std::env::var("TEST_MY_URL")
            .unwrap_or_else(|_| "mysql://root:root@localhost:3306/mysql".to_string());
        let pool = connect(&url).await.unwrap();

        sqlx::query("CREATE TEMPORARY TABLE t (id INT, val VARCHAR(255))")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO t VALUES (1, NULL), (2, 'hi')")
            .execute(&pool)
            .await
            .unwrap();

        let result = execute(&pool, "SELECT id, val FROM t ORDER BY id")
            .await
            .unwrap();

        assert_eq!(result.row_count, 2);
        assert_eq!(result.rows[0][1], None);
        assert_eq!(result.rows[1][1], Some("hi".to_string()));
    }

    #[tokio::test]
    #[ignore]
    async fn execute_insert_should_return_affected_row_count() {
        let url = std::env::var("TEST_MY_URL")
            .unwrap_or_else(|_| "mysql://root:root@localhost:3306/mysql".to_string());
        let pool = connect(&url).await.unwrap();

        sqlx::query("CREATE TEMPORARY TABLE t (id INT)")
            .execute(&pool)
            .await
            .unwrap();

        let result = execute(&pool, "INSERT INTO t VALUES (1), (2)")
            .await
            .unwrap();

        assert_eq!(result.row_count, 2);
        assert!(result.rows.is_empty());
    }
}

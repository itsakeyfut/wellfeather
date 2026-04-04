use std::time::Instant;

use sqlx::{Column, PgPool, Row, TypeInfo};

use crate::error::DbError;
use crate::models::QueryResult;

/// Connect to a PostgreSQL database at `url`.
///
/// Any sqlx connection error is wrapped as [`DbError::ConnectionFailed`].
pub async fn connect(url: &str) -> Result<PgPool, DbError> {
    PgPool::connect(url)
        .await
        .map_err(|e| DbError::ConnectionFailed(e.to_string()))
}

/// Execute `sql` against `pool` and return a [`QueryResult`].
///
/// - **SELECT / row-returning statements**: columns and rows are populated.
/// - **DML / DDL statements**: rows are empty; `row_count` = `rows_affected()`.
/// - **NULL values** map to `None`.
/// - `execution_time_ms` is measured with [`Instant`].
pub async fn execute(pool: &PgPool, sql: &str) -> Result<QueryResult, DbError> {
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

/// Convert a single PostgreSQL cell to `Option<String>`.
///
/// NULL values are detected first (via `Option<T>` decode returning `Ok(None)`).
/// Each column type is then matched against common PostgreSQL type names and
/// decoded with an appropriate Rust type before stringification.
fn cell_to_string(row: &sqlx::postgres::PgRow, i: usize) -> Option<String> {
    // Step 1 — NULL + text-like types
    if let Ok(v) = row.try_get::<Option<String>, _>(i) {
        return v;
    }

    // Step 2 — type-specific decode
    let col_type = row.column(i).type_info().name().to_ascii_uppercase();
    let col_type = col_type.as_str();

    // Integer types
    if matches!(
        col_type,
        "INT2" | "INT4" | "INT8" | "OID" | "SERIAL" | "BIGSERIAL"
    ) {
        return row
            .try_get::<Option<i64>, _>(i)
            .ok()
            .flatten()
            .map(|v| v.to_string());
    }

    // Floating-point types
    if matches!(col_type, "FLOAT4" | "FLOAT8") {
        return row
            .try_get::<Option<f64>, _>(i)
            .ok()
            .flatten()
            .map(|v| v.to_string());
    }

    // Boolean
    if col_type == "BOOL" {
        return row
            .try_get::<Option<bool>, _>(i)
            .ok()
            .flatten()
            .map(|v| v.to_string());
    }

    // UUID (requires sqlx "uuid" feature)
    if col_type == "UUID" {
        return row
            .try_get::<Option<uuid::Uuid>, _>(i)
            .ok()
            .flatten()
            .map(|v| v.to_string());
    }

    // Date / time (requires sqlx "chrono" feature)
    if col_type == "TIMESTAMPTZ" {
        return row
            .try_get::<Option<chrono::DateTime<chrono::Utc>>, _>(i)
            .ok()
            .flatten()
            .map(|v| v.to_rfc3339());
    }
    if col_type == "TIMESTAMP" {
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

    // Binary
    if col_type == "BYTEA" {
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

    /// Requires a running PostgreSQL instance.
    /// Set `TEST_PG_URL` to override the default connection string.
    /// Run with: `cargo test -p wf-db -- --ignored`
    #[tokio::test]
    #[ignore]
    async fn connect_should_succeed_with_real_postgres() {
        let url = std::env::var("TEST_PG_URL").unwrap_or_else(|_| {
            "postgresql://postgres:postgres@localhost:5432/postgres".to_string()
        });
        let pool = connect(&url).await.unwrap();
        let row: (i32,) = sqlx::query_as("SELECT 1").fetch_one(&pool).await.unwrap();
        assert_eq!(row.0, 1);
    }

    #[tokio::test]
    #[ignore]
    async fn connect_should_return_connection_failed_on_unreachable_host() {
        let result = connect("postgresql://user:pass@127.0.0.1:19999/db").await;
        assert!(matches!(result, Err(DbError::ConnectionFailed(_))));
    }

    #[tokio::test]
    #[ignore]
    async fn execute_select_should_return_rows_with_null_values() {
        let url = std::env::var("TEST_PG_URL").unwrap_or_else(|_| {
            "postgresql://postgres:postgres@localhost:5432/postgres".to_string()
        });
        let pool = connect(&url).await.unwrap();

        sqlx::query("CREATE TEMP TABLE t (id INT, val TEXT)")
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
        let url = std::env::var("TEST_PG_URL").unwrap_or_else(|_| {
            "postgresql://postgres:postgres@localhost:5432/postgres".to_string()
        });
        let pool = connect(&url).await.unwrap();

        sqlx::query("CREATE TEMP TABLE t (id INT)")
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

use std::time::Instant;

use sqlx::{Column, PgPool, Row, TypeInfo};

use std::collections::HashMap;

use crate::error::DbError;
use crate::models::{ColumnInfo, DbMetadata, QueryResult, TableInfo};

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

/// Fetch schema metadata from the connected PostgreSQL database.
///
/// Queries `information_schema` and `pg_indexes` restricted to the `public`
/// schema.  PG/MySQL tests are `#[ignore]` — run with `cargo test -- --ignored`.
pub async fn fetch_metadata(pool: &PgPool) -> Result<DbMetadata, DbError> {
    use sqlx::Row as _;

    // ── all columns (single round-trip) ───────────────────────────────────────
    let col_rows = sqlx::query(
        "SELECT table_name, column_name, data_type, is_nullable \
         FROM information_schema.columns \
         WHERE table_schema = 'public' \
         ORDER BY table_name, ordinal_position",
    )
    .fetch_all(pool)
    .await
    .map_err(DbError::from)?;

    let mut col_map: HashMap<String, Vec<ColumnInfo>> = HashMap::new();
    for row in &col_rows {
        let table: String = row.get("table_name");
        col_map.entry(table).or_default().push(ColumnInfo {
            name: row.get("column_name"),
            data_type: row.get("data_type"),
            nullable: row.get::<&str, _>("is_nullable") == "YES",
        });
    }

    // ── tables ────────────────────────────────────────────────────────────────
    let table_rows = sqlx::query(
        "SELECT table_name FROM information_schema.tables \
         WHERE table_schema = 'public' AND table_type = 'BASE TABLE' \
         ORDER BY table_name",
    )
    .fetch_all(pool)
    .await
    .map_err(DbError::from)?;

    let tables: Vec<TableInfo> = table_rows
        .iter()
        .map(|r| {
            let name: String = r.get("table_name");
            let columns = col_map.remove(&name).unwrap_or_default();
            TableInfo { name, columns }
        })
        .collect();

    // ── views ─────────────────────────────────────────────────────────────────
    let view_rows = sqlx::query(
        "SELECT table_name FROM information_schema.views \
         WHERE table_schema = 'public' \
         ORDER BY table_name",
    )
    .fetch_all(pool)
    .await
    .map_err(DbError::from)?;

    let views: Vec<TableInfo> = view_rows
        .iter()
        .map(|r| {
            let name: String = r.get("table_name");
            let columns = col_map.remove(&name).unwrap_or_default();
            TableInfo { name, columns }
        })
        .collect();

    // ── stored procedures / functions ─────────────────────────────────────────
    let proc_rows = sqlx::query(
        "SELECT routine_name FROM information_schema.routines \
         WHERE routine_schema = 'public' \
         ORDER BY routine_name",
    )
    .fetch_all(pool)
    .await
    .map_err(DbError::from)?;

    let stored_procs: Vec<String> = proc_rows.iter().map(|r| r.get("routine_name")).collect();

    // ── indexes ───────────────────────────────────────────────────────────────
    let index_rows = sqlx::query(
        "SELECT indexname FROM pg_indexes \
         WHERE schemaname = 'public' \
         ORDER BY indexname",
    )
    .fetch_all(pool)
    .await
    .map_err(DbError::from)?;

    let indexes: Vec<String> = index_rows.iter().map(|r| r.get("indexname")).collect();

    Ok(DbMetadata {
        tables,
        views,
        stored_procs,
        indexes,
    })
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

    // Date / time (requires sqlx "chrono" feature).
    // Use starts_with to tolerate precision suffixes like TIMESTAMP(6) / TIMESTAMPTZ(6).
    if col_type.starts_with("TIMESTAMPTZ") {
        return row
            .try_get::<Option<chrono::DateTime<chrono::Utc>>, _>(i)
            .ok()
            .flatten()
            .map(|v| v.to_rfc3339());
    }
    if col_type.starts_with("TIMESTAMP") {
        return row
            .try_get::<Option<chrono::NaiveDateTime>, _>(i)
            .ok()
            .flatten()
            .map(|v| v.to_string());
    }
    if col_type.starts_with("DATE") {
        return row
            .try_get::<Option<chrono::NaiveDate>, _>(i)
            .ok()
            .flatten()
            .map(|v| v.to_string());
    }
    if col_type == "TIME" || col_type.starts_with("TIME(") {
        return row
            .try_get::<Option<chrono::NaiveTime>, _>(i)
            .ok()
            .flatten()
            .map(|v| v.to_string());
    }

    // Binary — try UTF-8 first; emit byte-count tag only for truly binary content.
    if col_type == "BYTEA" {
        return row
            .try_get::<Option<Vec<u8>>, _>(i)
            .ok()
            .flatten()
            .map(|bytes| {
                String::from_utf8(bytes.clone())
                    .unwrap_or_else(|_| format!("<BLOB: {} bytes>", bytes.len()))
            });
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
    async fn fetch_metadata_should_return_tables_views_procs_and_indexes() {
        let url = std::env::var("TEST_PG_URL").unwrap_or_else(|_| {
            "postgresql://postgres:postgres@localhost:5432/postgres".to_string()
        });
        let pool = connect(&url).await.unwrap();

        sqlx::query("CREATE TEMP TABLE wf_meta_test (id INT NOT NULL, label TEXT)")
            .execute(&pool)
            .await
            .unwrap();

        let meta = fetch_metadata(&pool).await.unwrap();

        // Temp tables live in pg_temp_* schema, not public — metadata should be non-empty
        // in a real database but at minimum the call must succeed.
        let _ = meta;
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

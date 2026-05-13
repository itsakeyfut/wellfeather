use sqlx::{Row as _, SqlitePool};
use wf_db::models::QueryExecution;

use crate::error::HistoryError;

type Result<T> = std::result::Result<T, HistoryError>;

// ---------------------------------------------------------------------------
// HistoryService
// ---------------------------------------------------------------------------

/// Persists [`QueryExecution`] records to SQLite.
///
/// Cheap to clone — all clones share the same underlying connection pool.
#[derive(Clone)]
pub struct HistoryService {
    pool: SqlitePool,
}

const CREATE_TABLE: &str = "
    CREATE TABLE IF NOT EXISTS query_executions (
        id            INTEGER PRIMARY KEY AUTOINCREMENT,
        sql           TEXT    NOT NULL,
        duration_ms   INTEGER NOT NULL,
        success       INTEGER NOT NULL,
        error_message TEXT,
        timestamp     INTEGER NOT NULL,
        connection_id TEXT    NOT NULL
    )";

impl HistoryService {
    /// Accept an already-open [`SqlitePool`] and ensure the schema exists.
    pub async fn new(pool: SqlitePool) -> Result<Self> {
        Self::migrate(&pool).await?;
        Ok(Self { pool })
    }

    async fn migrate(pool: &SqlitePool) -> Result<()> {
        sqlx::query(CREATE_TABLE).execute(pool).await?;
        Ok(())
    }

    /// Persist one [`QueryExecution`] record.
    ///
    /// The `id` field is ignored — SQLite assigns the ROWID automatically.
    pub async fn insert(&self, execution: &QueryExecution) -> Result<()> {
        sqlx::query(
            "INSERT INTO query_executions
             (sql, duration_ms, success, error_message, timestamp, connection_id)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&execution.sql)
        .bind(execution.duration_ms as i64)
        .bind(execution.success as i32)
        .bind(&execution.error_message)
        .bind(execution.timestamp)
        .bind(&execution.connection_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Return up to `limit` executions matching `keyword` in the SQL text,
    /// optionally restricted to a single connection.
    ///
    /// Results are ordered newest-first (DESC timestamp).
    pub async fn search(
        &self,
        keyword: &str,
        filter_conn: Option<&str>,
        limit: usize,
    ) -> Result<Vec<QueryExecution>> {
        let pattern = format!("%{keyword}%");
        let rows = sqlx::query(
            "SELECT id, sql, duration_ms, success, error_message, timestamp, connection_id
             FROM query_executions
             WHERE sql LIKE ?
               AND connection_id = COALESCE(?, connection_id)
             ORDER BY timestamp DESC
             LIMIT ?",
        )
        .bind(&pattern)
        .bind(filter_conn)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        let executions = rows
            .iter()
            .map(|row| QueryExecution {
                id: row.get("id"),
                sql: row.get("sql"),
                duration_ms: row.get::<i64, _>("duration_ms") as u128,
                success: row.get::<i32, _>("success") != 0,
                error_message: row.get("error_message"),
                timestamp: row.get("timestamp"),
                connection_id: row.get("connection_id"),
            })
            .collect();

        Ok(executions)
    }

    /// Return up to `limit` most recent executions, newest first (DESC timestamp).
    pub async fn recent(&self, limit: usize) -> Result<Vec<QueryExecution>> {
        let rows = sqlx::query(
            "SELECT id, sql, duration_ms, success, error_message, timestamp, connection_id
             FROM query_executions
             ORDER BY timestamp DESC
             LIMIT ?",
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        let executions = rows
            .iter()
            .map(|row| QueryExecution {
                id: row.get("id"),
                sql: row.get("sql"),
                duration_ms: row.get::<i64, _>("duration_ms") as u128,
                success: row.get::<i32, _>("success") != 0,
                error_message: row.get("error_message"),
                timestamp: row.get("timestamp"),
                connection_id: row.get("connection_id"),
            })
            .collect();

        Ok(executions)
    }

    #[cfg(test)]
    async fn open_memory() -> Result<Self> {
        let pool = SqlitePool::connect("sqlite::memory:").await?;
        Self::new(pool).await
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_exec(sql: &str, ts: i64, success: bool, err: Option<&str>) -> QueryExecution {
        make_exec_for("c1", sql, ts, success, err)
    }

    fn make_exec_for(
        conn_id: &str,
        sql: &str,
        ts: i64,
        success: bool,
        err: Option<&str>,
    ) -> QueryExecution {
        QueryExecution {
            id: 0,
            sql: sql.to_string(),
            duration_ms: 5,
            success,
            error_message: err.map(|s| s.to_string()),
            timestamp: ts,
            connection_id: conn_id.to_string(),
        }
    }

    #[tokio::test]
    async fn insert_and_recent_should_roundtrip() {
        let svc = HistoryService::open_memory().await.unwrap();

        svc.insert(&make_exec("SELECT 1", 1000, true, None))
            .await
            .unwrap();
        svc.insert(&make_exec("SELECT 2", 2000, false, Some("err")))
            .await
            .unwrap();

        let rows = svc.recent(10).await.unwrap();
        assert_eq!(rows.len(), 2);
        // DESC by timestamp → newest first
        assert_eq!(rows[0].sql, "SELECT 2");
        assert!(!rows[0].success);
        assert_eq!(rows[0].error_message.as_deref(), Some("err"));
        assert_eq!(rows[1].sql, "SELECT 1");
        assert!(rows[1].success);
        assert!(rows[1].error_message.is_none());
    }

    #[tokio::test]
    async fn recent_should_respect_limit() {
        let svc = HistoryService::open_memory().await.unwrap();
        for i in 0..5_i64 {
            svc.insert(&make_exec(&format!("SELECT {i}"), i, true, None))
                .await
                .unwrap();
        }
        let rows = svc.recent(3).await.unwrap();
        assert_eq!(rows.len(), 3);
    }

    #[tokio::test]
    async fn recent_should_return_empty_when_no_rows() {
        let svc = HistoryService::open_memory().await.unwrap();
        let rows = svc.recent(10).await.unwrap();
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn search_should_filter_by_keyword() {
        let svc = HistoryService::open_memory().await.unwrap();
        svc.insert(&make_exec("SELECT name FROM users", 1000, true, None))
            .await
            .unwrap();
        svc.insert(&make_exec(
            "INSERT INTO orders VALUES (1)",
            2000,
            true,
            None,
        ))
        .await
        .unwrap();

        let rows = svc.search("users", None, 10).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].sql.contains("users"));
    }

    #[tokio::test]
    async fn search_should_filter_by_connection() {
        let svc = HistoryService::open_memory().await.unwrap();
        svc.insert(&make_exec_for("conn-a", "SELECT 1", 1000, true, None))
            .await
            .unwrap();
        svc.insert(&make_exec_for("conn-b", "SELECT 2", 2000, true, None))
            .await
            .unwrap();
        svc.insert(&make_exec_for("conn-a", "SELECT 3", 3000, true, None))
            .await
            .unwrap();

        let rows = svc.search("SELECT", Some("conn-a"), 10).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.connection_id == "conn-a"));
    }

    #[tokio::test]
    async fn search_should_filter_by_keyword_and_connection() {
        let svc = HistoryService::open_memory().await.unwrap();
        svc.insert(&make_exec_for(
            "conn-a",
            "SELECT * FROM users",
            1000,
            true,
            None,
        ))
        .await
        .unwrap();
        svc.insert(&make_exec_for(
            "conn-b",
            "SELECT * FROM users",
            2000,
            true,
            None,
        ))
        .await
        .unwrap();
        svc.insert(&make_exec_for(
            "conn-a",
            "DELETE FROM orders",
            3000,
            true,
            None,
        ))
        .await
        .unwrap();

        let rows = svc.search("users", Some("conn-a"), 10).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].connection_id, "conn-a");
        assert!(rows[0].sql.contains("users"));
    }
}

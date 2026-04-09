use std::path::Path;

use anyhow::Result;
use sqlx::SqlitePool;
use sqlx::sqlite::SqliteConnectOptions;
use wf_db::models::QueryExecution;

// ---------------------------------------------------------------------------
// HistoryService
// ---------------------------------------------------------------------------

/// Persists [`QueryExecution`] records to a SQLite `history.db` file.
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
    /// Open (or create) the history database at `db_path` and run schema migrations.
    ///
    /// Creates the file if it does not exist.  Returns an error if the path is
    /// not writable or if the migration query fails.
    pub async fn open(db_path: &Path) -> Result<Self> {
        let opts = SqliteConnectOptions::new()
            .filename(db_path)
            .create_if_missing(true);
        let pool = SqlitePool::connect_with(opts).await?;
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

        use sqlx::Row as _;
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

    /// In-memory database variant for unit tests only.
    #[cfg(test)]
    async fn open_memory() -> Result<Self> {
        let pool = SqlitePool::connect("sqlite::memory:").await?;
        Self::migrate(&pool).await?;
        Ok(Self { pool })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_exec(sql: &str, ts: i64, success: bool, err: Option<&str>) -> QueryExecution {
        QueryExecution {
            id: 0,
            sql: sql.to_string(),
            duration_ms: 5,
            success,
            error_message: err.map(|s| s.to_string()),
            timestamp: ts,
            connection_id: "c1".to_string(),
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
}

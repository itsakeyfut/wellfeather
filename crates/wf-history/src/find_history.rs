use std::path::Path;

use anyhow::Result;
use sqlx::SqlitePool;
use sqlx::sqlite::SqliteConnectOptions;

/// Persists find/replace bar search terms to a SQLite table in `history.db`.
///
/// Cheap to clone — all clones share the same underlying connection pool.
#[derive(Clone)]
pub struct FindHistoryService {
    pool: SqlitePool,
}

const CREATE_TABLE: &str = "
    CREATE TABLE IF NOT EXISTS find_history (
        id    INTEGER PRIMARY KEY AUTOINCREMENT,
        query TEXT    NOT NULL,
        kind  TEXT    NOT NULL CHECK(kind IN ('find','replace')),
        UNIQUE(query, kind)
    )";

impl FindHistoryService {
    /// Open (or create) the history database at `db_path` and run schema migrations.
    ///
    /// Shares the same file as [`crate::service::HistoryService`].
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

    /// Store a search term.  Silently ignores duplicates (INSERT OR IGNORE).
    pub async fn save(&self, kind: &str, query: &str) -> Result<()> {
        sqlx::query("INSERT OR IGNORE INTO find_history (query, kind) VALUES (?, ?)")
            .bind(query)
            .bind(kind)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Return up to `limit` most recently inserted unique terms, newest first.
    pub async fn get(&self, kind: &str, limit: usize) -> Result<Vec<String>> {
        let rows =
            sqlx::query("SELECT query FROM find_history WHERE kind = ? ORDER BY id DESC LIMIT ?")
                .bind(kind)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?;

        use sqlx::Row as _;
        Ok(rows.iter().map(|r| r.get("query")).collect())
    }

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

    #[tokio::test]
    async fn save_and_get_should_roundtrip() {
        let svc = FindHistoryService::open_memory().await.unwrap();
        svc.save("find", "SELECT").await.unwrap();
        svc.save("find", "FROM users").await.unwrap();
        let items = svc.get("find", 10).await.unwrap();
        assert_eq!(items, vec!["FROM users", "SELECT"]);
    }

    #[tokio::test]
    async fn save_should_ignore_duplicate() {
        let svc = FindHistoryService::open_memory().await.unwrap();
        svc.save("find", "SELECT").await.unwrap();
        svc.save("find", "SELECT").await.unwrap();
        let items = svc.get("find", 10).await.unwrap();
        assert_eq!(items.len(), 1);
    }

    #[tokio::test]
    async fn get_should_respect_limit() {
        let svc = FindHistoryService::open_memory().await.unwrap();
        for i in 0..5 {
            svc.save("find", &format!("query {i}")).await.unwrap();
        }
        let items = svc.get("find", 3).await.unwrap();
        assert_eq!(items.len(), 3);
    }

    #[tokio::test]
    async fn find_and_replace_histories_should_be_separate() {
        let svc = FindHistoryService::open_memory().await.unwrap();
        svc.save("find", "SELECT").await.unwrap();
        svc.save("replace", "hello").await.unwrap();
        assert_eq!(svc.get("find", 10).await.unwrap(), vec!["SELECT"]);
        assert_eq!(svc.get("replace", 10).await.unwrap(), vec!["hello"]);
    }
}

use anyhow::Context as _;
use sqlx::{Row as _, SqlitePool};

/// A single SQL Editor tab persisted across app restarts.
#[derive(Debug, Clone)]
pub struct TabSessionEntry {
    pub id: String,
    pub title: String,
    pub query_text: String,
}

const CREATE_TABS: &str = "
    CREATE TABLE IF NOT EXISTS session_tabs (
        sort_order  INTEGER NOT NULL,
        id          TEXT    NOT NULL,
        title       TEXT    NOT NULL,
        query_text  TEXT    NOT NULL DEFAULT '',
        is_active   INTEGER NOT NULL DEFAULT 0
    )
";

const CREATE_STATE: &str = "
    CREATE TABLE IF NOT EXISTS session_state (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    )
";

/// Persists tab state and last-query text to SQLite.
///
/// Cheap to clone — all clones share the same underlying connection pool.
#[derive(Clone)]
pub struct SessionService {
    pool: SqlitePool,
}

impl SessionService {
    /// Accept an already-open [`SqlitePool`] and ensure the schema exists.
    pub async fn new(pool: SqlitePool) -> anyhow::Result<Self> {
        sqlx::query(CREATE_TABS)
            .execute(&pool)
            .await
            .context("failed to create session_tabs table")?;
        sqlx::query(CREATE_STATE)
            .execute(&pool)
            .await
            .context("failed to create session_state table")?;
        Ok(Self { pool })
    }

    /// Persist the editor tabs. Replaces all previously saved tabs.
    ///
    /// `active_index` is the index within `tabs` (SQL Editor tabs only) that was active.
    pub async fn save_tabs(
        &self,
        active_index: usize,
        tabs: &[TabSessionEntry],
    ) -> anyhow::Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM session_tabs")
            .execute(&mut *tx)
            .await?;
        for (i, tab) in tabs.iter().enumerate() {
            let is_active = if i == active_index { 1i32 } else { 0i32 };
            sqlx::query(
                "INSERT INTO session_tabs (sort_order, id, title, query_text, is_active)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(i as i64)
            .bind(&tab.id)
            .bind(&tab.title)
            .bind(&tab.query_text)
            .bind(is_active)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Restore tabs from the previous session.
    ///
    /// Returns `None` when no tabs have been saved yet.
    pub async fn restore_tabs(&self) -> anyhow::Result<Option<(usize, Vec<TabSessionEntry>)>> {
        let rows = sqlx::query(
            "SELECT sort_order, id, title, query_text, is_active
             FROM session_tabs ORDER BY sort_order ASC",
        )
        .fetch_all(&self.pool)
        .await?;

        if rows.is_empty() {
            return Ok(None);
        }

        let mut active_index = 0usize;
        let entries: Vec<TabSessionEntry> = rows
            .iter()
            .enumerate()
            .map(|(i, row)| {
                let is_active: i64 = row.try_get("is_active").unwrap_or(0);
                if is_active != 0 {
                    active_index = i;
                }
                Ok(TabSessionEntry {
                    id: row.try_get("id")?,
                    title: row.try_get("title")?,
                    query_text: row.try_get("query_text")?,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        Ok(Some((active_index, entries)))
    }

    /// Persist the last active editor query text.
    pub async fn save_last_query(&self, query: &str) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO session_state (key, value) VALUES ('last_query', ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(query)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Restore the last active editor query text.
    ///
    /// Returns `None` when no query has been saved or it is empty.
    pub async fn restore_last_query(&self) -> anyhow::Result<Option<String>> {
        let value: Option<String> =
            sqlx::query_scalar("SELECT value FROM session_state WHERE key = 'last_query'")
                .fetch_optional(&self.pool)
                .await?;
        Ok(value.filter(|s| !s.is_empty()))
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    async fn open_memory() -> SessionService {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        SessionService::new(pool).await.unwrap()
    }

    fn make_tab(id: &str, title: &str, query: &str) -> TabSessionEntry {
        TabSessionEntry {
            id: id.to_string(),
            title: title.to_string(),
            query_text: query.to_string(),
        }
    }

    #[tokio::test]
    async fn session_service_should_restore_tabs_after_save() {
        let svc = open_memory().await;
        let tabs = vec![
            make_tab("t1", "Query 1", "SELECT 1"),
            make_tab("t2", "Query 2", "SELECT 2"),
        ];
        svc.save_tabs(1, &tabs).await.unwrap();

        let (active, restored) = svc.restore_tabs().await.unwrap().expect("should restore");
        assert_eq!(active, 1);
        assert_eq!(restored.len(), 2);
        assert_eq!(restored[0].id, "t1");
        assert_eq!(restored[1].query_text, "SELECT 2");
    }

    #[tokio::test]
    async fn session_service_restore_tabs_should_return_none_when_empty() {
        let svc = open_memory().await;
        assert!(svc.restore_tabs().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn session_service_save_tabs_should_replace_previous() {
        let svc = open_memory().await;
        svc.save_tabs(0, &[make_tab("t1", "Q1", "SELECT 1")])
            .await
            .unwrap();
        svc.save_tabs(0, &[make_tab("t2", "Q2", "SELECT 2")])
            .await
            .unwrap();

        let (_, restored) = svc.restore_tabs().await.unwrap().expect("should restore");
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].id, "t2");
    }

    #[tokio::test]
    async fn session_service_should_save_and_restore_last_query() {
        let svc = open_memory().await;
        svc.save_last_query("SELECT * FROM users").await.unwrap();
        let q = svc.restore_last_query().await.unwrap();
        assert_eq!(q, Some("SELECT * FROM users".to_string()));
    }

    #[tokio::test]
    async fn session_service_restore_last_query_should_return_none_when_absent() {
        let svc = open_memory().await;
        assert!(svc.restore_last_query().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn session_service_restore_last_query_should_return_none_for_empty_string() {
        let svc = open_memory().await;
        svc.save_last_query("").await.unwrap();
        assert!(svc.restore_last_query().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn session_service_save_last_query_should_overwrite_previous() {
        let svc = open_memory().await;
        svc.save_last_query("SELECT 1").await.unwrap();
        svc.save_last_query("SELECT 2").await.unwrap();
        let q = svc.restore_last_query().await.unwrap();
        assert_eq!(q, Some("SELECT 2".to_string()));
    }
}

use anyhow::Context as _;
use sqlx::{Row as _, SqlitePool};

const CREATE_SNIPPETS: &str = "
    CREATE TABLE IF NOT EXISTS snippets (
        id            TEXT    PRIMARY KEY,
        name          TEXT    NOT NULL,
        comment       TEXT    NOT NULL DEFAULT '',
        connection_id TEXT,
        sql           TEXT    NOT NULL,
        created_at    TEXT    NOT NULL,
        sort_order    INTEGER NOT NULL DEFAULT 0
    )
";

const CREATE_BAR_POSITION: &str = "
    CREATE TABLE IF NOT EXISTS snippet_bar_position (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        x  REAL    NOT NULL DEFAULT 0.0,
        y  REAL    NOT NULL DEFAULT 100.0
    )
";

/// A named SQL snippet persisted across app restarts.
#[derive(Debug, Clone)]
pub struct SnippetEntry {
    pub id: String,
    pub name: String,
    pub comment: String,
    pub connection_id: Option<String>,
    pub sql: String,
    pub created_at: String,
    pub sort_order: i64,
}

/// Persists snippet entries and bar position to SQLite.
///
/// Cheap to clone — all clones share the same underlying connection pool.
#[derive(Clone)]
pub struct SnippetRepository {
    pool: SqlitePool,
}

impl SnippetRepository {
    /// Accept an already-open [`SqlitePool`] and ensure the schema exists.
    pub async fn new(pool: SqlitePool) -> anyhow::Result<Self> {
        sqlx::query(CREATE_SNIPPETS)
            .execute(&pool)
            .await
            .context("failed to create snippets table")?;
        sqlx::query(CREATE_BAR_POSITION)
            .execute(&pool)
            .await
            .context("failed to create snippet_bar_position table")?;
        Ok(Self { pool })
    }

    /// Persist a new snippet.
    pub async fn add(&self, entry: &SnippetEntry) -> anyhow::Result<()> {
        let order = self.next_sort_order().await?;
        sqlx::query(
            "INSERT INTO snippets (id, name, comment, connection_id, sql, created_at, sort_order)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&entry.id)
        .bind(&entry.name)
        .bind(&entry.comment)
        .bind(&entry.connection_id)
        .bind(&entry.sql)
        .bind(&entry.created_at)
        .bind(order)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Return snippets visible for the given `connection_id`.
    ///
    /// - `None` → global snippets only (`connection_id IS NULL`).
    /// - `Some(id)` → global snippets plus snippets scoped to that connection.
    pub async fn list(&self, connection_id: Option<&str>) -> anyhow::Result<Vec<SnippetEntry>> {
        let rows = match connection_id {
            None => {
                sqlx::query(
                    "SELECT id, name, comment, connection_id, sql, created_at, sort_order
                     FROM snippets WHERE connection_id IS NULL
                     ORDER BY sort_order ASC, created_at ASC",
                )
                .fetch_all(&self.pool)
                .await?
            }
            Some(id) => {
                sqlx::query(
                    "SELECT id, name, comment, connection_id, sql, created_at, sort_order
                     FROM snippets WHERE connection_id IS NULL OR connection_id = ?
                     ORDER BY sort_order ASC, created_at ASC",
                )
                .bind(id)
                .fetch_all(&self.pool)
                .await?
            }
        };
        rows.iter().map(row_to_entry).collect()
    }

    /// Update the name of an existing snippet.
    pub async fn rename(&self, id: &str, new_name: &str) -> anyhow::Result<()> {
        sqlx::query("UPDATE snippets SET name = ? WHERE id = ?")
            .bind(new_name)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Update comment and SQL of an existing snippet.
    pub async fn update(&self, id: &str, comment: &str, sql: &str) -> anyhow::Result<()> {
        sqlx::query("UPDATE snippets SET comment = ?, sql = ? WHERE id = ?")
            .bind(comment)
            .bind(sql)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Update the comment of an existing snippet.
    pub async fn update_comment(&self, id: &str, comment: &str) -> anyhow::Result<()> {
        sqlx::query("UPDATE snippets SET comment = ? WHERE id = ?")
            .bind(comment)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Delete a snippet by id.
    pub async fn delete(&self, id: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM snippets WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Read the last-saved Snippet Bar position. Returns `(0.0, 100.0)` when unset.
    pub async fn get_bar_position(&self) -> anyhow::Result<(f32, f32)> {
        let row = sqlx::query("SELECT x, y FROM snippet_bar_position WHERE id = 1")
            .fetch_optional(&self.pool)
            .await?;
        match row {
            Some(r) => Ok((r.try_get("x")?, r.try_get("y")?)),
            None => Ok((0.0, 100.0)),
        }
    }

    /// Persist the Snippet Bar position (upsert single-row sentinel).
    pub async fn set_bar_position(&self, x: f32, y: f32) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO snippet_bar_position (id, x, y) VALUES (1, ?, ?)
             ON CONFLICT(id) DO UPDATE SET x = excluded.x, y = excluded.y",
        )
        .bind(x)
        .bind(y)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Returns the next sequential number for auto-named snippets ("Query N").
    /// Finds the highest N already used and returns N+1, so numbers never repeat
    /// even after deletion.
    pub async fn next_query_number(&self) -> anyhow::Result<u64> {
        let names: Vec<String> =
            sqlx::query_scalar("SELECT name FROM snippets WHERE name LIKE 'Query %'")
                .fetch_all(&self.pool)
                .await?;
        let max = names
            .iter()
            .filter_map(|n| n.strip_prefix("Query ").and_then(|s| s.parse::<u64>().ok()))
            .max()
            .unwrap_or(0);
        Ok(max + 1)
    }

    async fn next_sort_order(&self) -> anyhow::Result<i64> {
        let max: Option<i64> = sqlx::query_scalar("SELECT MAX(sort_order) FROM snippets")
            .fetch_one(&self.pool)
            .await?;
        Ok(max.unwrap_or(-1) + 1)
    }
}

// ── Row → model ───────────────────────────────────────────────────────────────

fn row_to_entry(row: &sqlx::sqlite::SqliteRow) -> anyhow::Result<SnippetEntry> {
    Ok(SnippetEntry {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        comment: row
            .try_get::<Option<String>, _>("comment")?
            .unwrap_or_default(),
        connection_id: row.try_get("connection_id")?,
        sql: row.try_get("sql")?,
        created_at: row.try_get("created_at")?,
        sort_order: row.try_get("sort_order").unwrap_or(0),
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    async fn open_memory() -> SnippetRepository {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        SnippetRepository::new(pool).await.unwrap()
    }

    fn make_entry(id: &str, name: &str, sql: &str) -> SnippetEntry {
        SnippetEntry {
            id: id.to_string(),
            name: name.to_string(),
            comment: String::new(),
            connection_id: None,
            sql: sql.to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            sort_order: 0,
        }
    }

    #[tokio::test]
    async fn snippet_repository_should_add_and_list() {
        let repo = open_memory().await;
        repo.add(&make_entry("b1", "My Query", "SELECT 1"))
            .await
            .unwrap();
        let items = repo.list(None).await.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "b1");
        assert_eq!(items[0].name, "My Query");
        assert_eq!(items[0].sql, "SELECT 1");
    }

    #[tokio::test]
    async fn snippet_repository_list_should_return_empty_when_no_snippets() {
        let repo = open_memory().await;
        assert!(repo.list(None).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn snippet_repository_list_none_should_return_only_global_snippets() {
        let repo = open_memory().await;
        repo.add(&make_entry("b1", "Global", "SELECT 1"))
            .await
            .unwrap();
        let mut per_conn = make_entry("b2", "PerConn", "SELECT 2");
        per_conn.connection_id = Some("conn1".to_string());
        repo.add(&per_conn).await.unwrap();

        let items = repo.list(None).await.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "b1");
    }

    #[tokio::test]
    async fn snippet_repository_list_some_should_return_global_and_per_connection() {
        let repo = open_memory().await;
        repo.add(&make_entry("b1", "Global", "SELECT 1"))
            .await
            .unwrap();
        let mut per_conn1 = make_entry("b2", "Conn1Query", "SELECT 2");
        per_conn1.connection_id = Some("conn1".to_string());
        repo.add(&per_conn1).await.unwrap();
        let mut per_conn2 = make_entry("b3", "Conn2Query", "SELECT 3");
        per_conn2.connection_id = Some("conn2".to_string());
        repo.add(&per_conn2).await.unwrap();

        let items = repo.list(Some("conn1")).await.unwrap();
        assert_eq!(items.len(), 2);
        let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
        assert!(ids.contains(&"b1"));
        assert!(ids.contains(&"b2"));
        assert!(!ids.contains(&"b3"));
    }

    #[tokio::test]
    async fn snippet_repository_should_rename() {
        let repo = open_memory().await;
        repo.add(&make_entry("b1", "Old Name", "SELECT 1"))
            .await
            .unwrap();
        repo.rename("b1", "New Name").await.unwrap();
        let items = repo.list(None).await.unwrap();
        assert_eq!(items[0].name, "New Name");
    }

    #[tokio::test]
    async fn snippet_repository_should_update_comment() {
        let repo = open_memory().await;
        repo.add(&make_entry("b1", "My Query", "SELECT 1"))
            .await
            .unwrap();
        repo.update_comment("b1", "Monthly report").await.unwrap();
        let items = repo.list(None).await.unwrap();
        assert_eq!(items[0].comment, "Monthly report");
    }

    #[tokio::test]
    async fn snippet_repository_should_delete() {
        let repo = open_memory().await;
        repo.add(&make_entry("b1", "My Query", "SELECT 1"))
            .await
            .unwrap();
        repo.delete("b1").await.unwrap();
        assert!(repo.list(None).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn snippet_repository_add_should_increment_sort_order() {
        let repo = open_memory().await;
        repo.add(&make_entry("b1", "First", "SELECT 1"))
            .await
            .unwrap();
        repo.add(&make_entry("b2", "Second", "SELECT 2"))
            .await
            .unwrap();
        let items = repo.list(None).await.unwrap();
        assert!(items[0].sort_order < items[1].sort_order);
    }

    #[tokio::test]
    async fn snippet_repository_bar_position_should_default_to_zero() {
        let repo = open_memory().await;
        let (x, y) = repo.get_bar_position().await.unwrap();
        assert_eq!(x, 0.0);
        assert_eq!(y, 100.0);
    }

    #[tokio::test]
    async fn snippet_repository_bar_position_should_roundtrip() {
        let repo = open_memory().await;
        repo.set_bar_position(320.5, 200.0).await.unwrap();
        let (x, y) = repo.get_bar_position().await.unwrap();
        assert!((x - 320.5).abs() < 0.001);
        assert!((y - 200.0).abs() < 0.001);
    }

    #[tokio::test]
    async fn snippet_repository_bar_position_should_update_on_second_set() {
        let repo = open_memory().await;
        repo.set_bar_position(100.0, 200.0).await.unwrap();
        repo.set_bar_position(400.0, 300.0).await.unwrap();
        let (x, y) = repo.get_bar_position().await.unwrap();
        assert!((x - 400.0).abs() < 0.001);
        assert!((y - 300.0).abs() < 0.001);
    }
}

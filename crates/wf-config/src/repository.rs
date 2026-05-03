use anyhow::Context as _;
use sqlx::{Row as _, SqlitePool};

use crate::models::{ConnectionConfig, DbTypeName};

const CREATE_TABLE: &str = "
    CREATE TABLE IF NOT EXISTS connections (
        id                 TEXT    PRIMARY KEY,
        name               TEXT    NOT NULL,
        db_type            TEXT    NOT NULL,
        connection_string  TEXT,
        host               TEXT,
        port               INTEGER,
        user_name          TEXT,
        password_encrypted TEXT,
        database_name      TEXT,
        safe_dml           INTEGER NOT NULL DEFAULT 1,
        read_only          INTEGER NOT NULL DEFAULT 0,
        sort_order         INTEGER NOT NULL DEFAULT 0,
        created_at         INTEGER NOT NULL DEFAULT (unixepoch()),
        last_used_at       INTEGER
    )
";

/// Persists [`ConnectionConfig`] records to SQLite.
///
/// Cheap to clone — all clones share the same underlying connection pool.
#[derive(Clone)]
pub struct ConnectionRepository {
    pool: SqlitePool,
}

impl ConnectionRepository {
    /// Accept an already-open [`SqlitePool`] and ensure the schema exists.
    pub async fn new(pool: SqlitePool) -> anyhow::Result<Self> {
        sqlx::query(CREATE_TABLE)
            .execute(&pool)
            .await
            .context("failed to migrate connections table")?;
        Ok(Self { pool })
    }

    /// In-memory database (for tests only).
    pub async fn open_memory() -> anyhow::Result<Self> {
        let pool = SqlitePool::connect("sqlite::memory:").await?;
        Self::new(pool).await
    }

    /// Return all saved connections ordered by `sort_order`, then `created_at`.
    pub async fn all(&self) -> anyhow::Result<Vec<ConnectionConfig>> {
        let rows = sqlx::query(
            "SELECT id, name, db_type, connection_string, host, port, user_name,
                    password_encrypted, database_name, safe_dml, read_only
             FROM connections ORDER BY sort_order ASC, created_at ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(row_to_config).collect()
    }

    /// Return the connection with the given `id`, or `None` if not found.
    pub async fn find(&self, id: &str) -> anyhow::Result<Option<ConnectionConfig>> {
        let row = sqlx::query(
            "SELECT id, name, db_type, connection_string, host, port, user_name,
                    password_encrypted, database_name, safe_dml, read_only
             FROM connections WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|r| row_to_config(&r)).transpose()
    }

    /// Insert or update a connection.
    ///
    /// On conflict (same `id`), updates all fields **except** `safe_dml`, `read_only`,
    /// and `sort_order` so that per-connection behaviour flags are preserved across reconnects.
    /// Use [`update_flags`] to change those explicitly.
    pub async fn upsert(&self, cc: &ConnectionConfig) -> anyhow::Result<()> {
        let order = self.next_sort_order().await?;
        self.upsert_with_order(cc, order).await
    }

    async fn upsert_with_order(&self, cc: &ConnectionConfig, order: i64) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO connections
             (id, name, db_type, connection_string, host, port, user_name,
              password_encrypted, database_name, safe_dml, read_only, sort_order)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
               name               = excluded.name,
               db_type            = excluded.db_type,
               connection_string  = excluded.connection_string,
               host               = excluded.host,
               port               = excluded.port,
               user_name          = excluded.user_name,
               password_encrypted = excluded.password_encrypted,
               database_name      = excluded.database_name",
        )
        .bind(&cc.id)
        .bind(&cc.name)
        .bind(db_type_to_str(&cc.db_type))
        .bind(&cc.connection_string)
        .bind(&cc.host)
        .bind(cc.port.map(|p| p as i64))
        .bind(&cc.user)
        .bind(&cc.password_encrypted)
        .bind(&cc.database)
        .bind(cc.safe_dml as i32)
        .bind(cc.read_only as i32)
        .bind(order)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Delete the connection with the given `id`.
    pub async fn delete(&self, id: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM connections WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Return the most-recently-used connection (highest `last_used_at`), or `None`.
    pub async fn last_used(&self) -> anyhow::Result<Option<ConnectionConfig>> {
        let row = sqlx::query(
            "SELECT id, name, db_type, connection_string, host, port, user_name,
                    password_encrypted, database_name, safe_dml, read_only
             FROM connections WHERE last_used_at IS NOT NULL
             ORDER BY last_used_at DESC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await?;
        row.map(|r| row_to_config(&r)).transpose()
    }

    /// Update only the `safe_dml` and `read_only` flags for a connection.
    pub async fn update_flags(
        &self,
        id: &str,
        safe_dml: bool,
        read_only: bool,
    ) -> anyhow::Result<()> {
        sqlx::query("UPDATE connections SET safe_dml = ?, read_only = ? WHERE id = ?")
            .bind(safe_dml as i32)
            .bind(read_only as i32)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Record a successful connection by stamping `last_used_at = now()`.
    pub async fn touch_last_used(&self, id: &str) -> anyhow::Result<()> {
        sqlx::query("UPDATE connections SET last_used_at = unixepoch() WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn next_sort_order(&self) -> anyhow::Result<i64> {
        let max: Option<i64> = sqlx::query_scalar("SELECT MAX(sort_order) FROM connections")
            .fetch_one(&self.pool)
            .await?;
        Ok(max.unwrap_or(-1) + 1)
    }
}

// ── Row → model conversion ────────────────────────────────────────────────────

fn row_to_config(row: &sqlx::sqlite::SqliteRow) -> anyhow::Result<ConnectionConfig> {
    let db_type_str: String = row.try_get("db_type")?;
    let db_type = match db_type_str.as_str() {
        "mysql" => DbTypeName::MySQL,
        "sqlite" => DbTypeName::SQLite,
        _ => DbTypeName::PostgreSQL,
    };
    let port_i: Option<i64> = row.try_get("port")?;
    Ok(ConnectionConfig {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        db_type,
        connection_string: row.try_get("connection_string")?,
        host: row.try_get("host")?,
        port: port_i.map(|p| p as u16),
        user: row.try_get("user_name")?,
        password_encrypted: row.try_get("password_encrypted")?,
        database: row.try_get("database_name")?,
        safe_dml: row.try_get::<i64, _>("safe_dml")? != 0,
        read_only: row.try_get::<i64, _>("read_only")? != 0,
    })
}

fn db_type_to_str(dt: &DbTypeName) -> &'static str {
    match dt {
        DbTypeName::PostgreSQL => "postgresql",
        DbTypeName::MySQL => "mysql",
        DbTypeName::SQLite => "sqlite",
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::DbTypeName;

    fn make_conn(id: &str) -> ConnectionConfig {
        ConnectionConfig {
            id: id.to_string(),
            name: format!("conn-{id}"),
            db_type: DbTypeName::SQLite,
            connection_string: Some("sqlite::memory:".to_string()),
            host: None,
            port: None,
            user: None,
            password_encrypted: None,
            database: None,
            safe_dml: true,
            read_only: false,
        }
    }

    #[tokio::test]
    async fn repository_should_upsert_and_return_all() {
        let repo = ConnectionRepository::open_memory().await.unwrap();
        repo.upsert(&make_conn("c1")).await.unwrap();
        repo.upsert(&make_conn("c2")).await.unwrap();
        let all = repo.all().await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].id, "c1");
        assert_eq!(all[1].id, "c2");
    }

    #[tokio::test]
    async fn repository_upsert_should_not_duplicate() {
        let repo = ConnectionRepository::open_memory().await.unwrap();
        repo.upsert(&make_conn("c1")).await.unwrap();
        repo.upsert(&make_conn("c1")).await.unwrap();
        let all = repo.all().await.unwrap();
        assert_eq!(all.len(), 1);
    }

    #[tokio::test]
    async fn repository_upsert_should_preserve_flags_on_update() {
        let repo = ConnectionRepository::open_memory().await.unwrap();
        let mut cc = make_conn("c1");
        cc.safe_dml = false;
        cc.read_only = true;
        repo.upsert(&cc).await.unwrap();

        // Second upsert with defaults should NOT overwrite safe_dml/read_only.
        repo.upsert(&make_conn("c1")).await.unwrap();
        let found = repo.find("c1").await.unwrap().unwrap();
        assert!(!found.safe_dml);
        assert!(found.read_only);
    }

    #[tokio::test]
    async fn repository_find_should_return_none_for_unknown_id() {
        let repo = ConnectionRepository::open_memory().await.unwrap();
        assert!(repo.find("nope").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn repository_delete_should_remove_entry() {
        let repo = ConnectionRepository::open_memory().await.unwrap();
        repo.upsert(&make_conn("c1")).await.unwrap();
        repo.delete("c1").await.unwrap();
        assert!(repo.all().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn repository_last_used_should_return_most_recent() {
        let repo = ConnectionRepository::open_memory().await.unwrap();
        repo.upsert(&make_conn("c1")).await.unwrap();
        repo.upsert(&make_conn("c2")).await.unwrap();
        repo.touch_last_used("c1").await.unwrap();
        // small delay so the unixepoch() values differ
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        repo.touch_last_used("c2").await.unwrap();
        let last = repo.last_used().await.unwrap().unwrap();
        assert_eq!(last.id, "c2");
    }

    #[tokio::test]
    async fn repository_last_used_should_return_none_when_empty() {
        let repo = ConnectionRepository::open_memory().await.unwrap();
        assert!(repo.last_used().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn repository_update_flags_should_change_safe_dml_and_read_only() {
        let repo = ConnectionRepository::open_memory().await.unwrap();
        repo.upsert(&make_conn("c1")).await.unwrap();
        repo.update_flags("c1", false, true).await.unwrap();
        let found = repo.find("c1").await.unwrap().unwrap();
        assert!(!found.safe_dml);
        assert!(found.read_only);
    }
}

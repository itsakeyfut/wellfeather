use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::RwLock;

use anyhow::Result;
use sqlx::SqlitePool;
use sqlx::sqlite::SqliteConnectOptions;
use wf_db::models::DbMetadata;

// ---------------------------------------------------------------------------
// MetadataCache
// ---------------------------------------------------------------------------

/// In-memory metadata cache with SQLite persistence.
///
/// Memory is the primary store; SQLite is used for across-session durability.
/// `new()` is synchronous — the SQLite file is opened lazily on first use.
pub struct MetadataCache {
    memory: RwLock<HashMap<String, DbMetadata>>,
    db_path: PathBuf,
}

impl MetadataCache {
    /// Create a cache backed by the SQLite file at `db_path`.
    /// The file is created on first write if it does not exist.
    pub fn new(db_path: PathBuf) -> Self {
        Self {
            memory: RwLock::new(HashMap::new()),
            db_path,
        }
    }

    /// Persist `meta` for `conn_id`: write to memory then flush to SQLite.
    pub async fn store(&self, conn_id: &str, meta: DbMetadata) -> Result<()> {
        self.memory
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .insert(conn_id.to_string(), meta.clone());

        let pool = self.open_pool().await?;
        let json = serde_json::to_vec(&meta)?;
        sqlx::query("INSERT OR REPLACE INTO metadata_cache (conn_id, data) VALUES (?, ?)")
            .bind(conn_id)
            .bind(json.as_slice())
            .execute(&pool)
            .await?;
        Ok(())
    }

    /// Return cached metadata for `conn_id`.
    ///
    /// Returns the in-memory value if present; otherwise queries SQLite and
    /// populates the memory cache before returning.
    pub async fn load(&self, conn_id: &str) -> Option<DbMetadata> {
        if let Some(m) = self
            .memory
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .get(conn_id)
            .cloned()
        {
            return Some(m);
        }

        let pool = self.open_pool().await.ok()?;
        use sqlx::Row as _;
        let row = sqlx::query("SELECT data FROM metadata_cache WHERE conn_id = ?")
            .bind(conn_id)
            .fetch_optional(&pool)
            .await
            .ok()??;

        let data: Vec<u8> = row.get("data");
        let meta: DbMetadata = serde_json::from_slice(&data).ok()?;
        self.memory
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .insert(conn_id.to_string(), meta.clone());
        Some(meta)
    }

    /// Populate the in-memory cache from SQLite.  Call once at startup.
    ///
    /// Non-fatal: if the database file cannot be opened (e.g. first run), a
    /// warning is logged and `Ok(())` is returned.
    pub async fn preload_from_disk(&self) -> Result<()> {
        let pool = match self.open_pool().await {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("metadata cache not available: {e}");
                return Ok(());
            }
        };

        use sqlx::Row as _;
        let rows = sqlx::query("SELECT conn_id, data FROM metadata_cache")
            .fetch_all(&pool)
            .await?;

        let mut guard = self.memory.write().unwrap_or_else(|p| p.into_inner());
        for row in &rows {
            let conn_id: String = row.get("conn_id");
            let data: Vec<u8> = row.get("data");
            if let Ok(meta) = serde_json::from_slice::<DbMetadata>(&data) {
                guard.insert(conn_id, meta);
            }
        }
        Ok(())
    }

    // ── private ───────────────────────────────────────────────────────────────

    async fn open_pool(&self) -> Result<SqlitePool> {
        let opts = SqliteConnectOptions::new()
            .filename(&self.db_path)
            .create_if_missing(true);
        let pool = SqlitePool::connect_with(opts).await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS metadata_cache (
                 conn_id TEXT PRIMARY KEY,
                 data    BLOB NOT NULL
             )",
        )
        .execute(&pool)
        .await?;
        Ok(pool)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use wf_db::models::{ColumnInfo, TableInfo};

    fn make_meta(table: &str) -> DbMetadata {
        DbMetadata {
            tables: vec![TableInfo {
                name: table.to_string(),
                columns: vec![
                    ColumnInfo {
                        name: "id".to_string(),
                        data_type: "INTEGER".to_string(),
                        nullable: false,
                    },
                    ColumnInfo {
                        name: "name".to_string(),
                        data_type: "TEXT".to_string(),
                        nullable: true,
                    },
                ],
            }],
            views: vec![],
            stored_procs: vec![],
            indexes: vec!["idx_id".to_string()],
        }
    }

    #[tokio::test]
    async fn store_and_load_should_roundtrip_via_file() {
        let dir = tempfile::tempdir().unwrap();
        let cache = MetadataCache::new(dir.path().join("metadata.db"));
        let meta = make_meta("users");

        cache.store("conn-1", meta.clone()).await.unwrap();
        let loaded = cache.load("conn-1").await.unwrap();

        assert_eq!(loaded.tables.len(), 1);
        assert_eq!(loaded.tables[0].name, "users");
        assert_eq!(loaded.tables[0].columns.len(), 2);
        assert_eq!(loaded.indexes[0], "idx_id");
    }

    #[tokio::test]
    async fn load_should_fall_back_to_sqlite_after_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("metadata.db");

        // Instance A — store
        let cache_a = MetadataCache::new(path.clone());
        cache_a.store("conn-1", make_meta("orders")).await.unwrap();

        // Instance B — fresh memory, same file
        let cache_b = MetadataCache::new(path);
        let loaded = cache_b.load("conn-1").await.unwrap();

        assert_eq!(loaded.tables[0].name, "orders");
    }

    #[tokio::test]
    async fn preload_from_disk_should_populate_memory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("metadata.db");

        let cache_a = MetadataCache::new(path.clone());
        cache_a
            .store("conn-1", make_meta("products"))
            .await
            .unwrap();

        let cache_b = MetadataCache::new(path);
        cache_b.preload_from_disk().await.unwrap();

        // After preload, memory should have the entry — verify by checking
        // that a subsequent load returns without hitting SQLite (same result).
        let loaded = cache_b.load("conn-1").await.unwrap();
        assert_eq!(loaded.tables[0].name, "products");
    }

    #[tokio::test]
    async fn load_should_return_none_for_unknown_conn_id() {
        let dir = tempfile::tempdir().unwrap();
        let cache = MetadataCache::new(dir.path().join("metadata.db"));
        let result = cache.load("does-not-exist").await;
        assert!(result.is_none());
    }
}

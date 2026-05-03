use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use anyhow::Result;
use sqlx::SqlitePool;
use wf_db::models::DbMetadata;

// ---------------------------------------------------------------------------
// MetadataCache
// ---------------------------------------------------------------------------

/// In-memory metadata cache with SQLite persistence.
///
/// Memory is the primary store; SQLite is used for across-session durability.
/// Cheap to clone — all clones share the same pool and in-memory map.
#[derive(Clone)]
pub struct MetadataCache {
    memory: Arc<RwLock<HashMap<String, DbMetadata>>>,
    pool: SqlitePool,
}

const CREATE_TABLE: &str = "
    CREATE TABLE IF NOT EXISTS metadata_cache (
        conn_id TEXT PRIMARY KEY,
        data    BLOB NOT NULL
    )";

impl MetadataCache {
    /// Accept an already-open [`SqlitePool`] and ensure the schema exists.
    pub async fn new(pool: SqlitePool) -> Result<Self> {
        sqlx::query(CREATE_TABLE).execute(&pool).await?;
        Ok(Self {
            memory: Arc::new(RwLock::new(HashMap::new())),
            pool,
        })
    }

    /// Persist `meta` for `conn_id`: write to memory then flush to SQLite.
    pub async fn store(&self, conn_id: &str, meta: DbMetadata) -> Result<()> {
        self.memory
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .insert(conn_id.to_string(), meta.clone());

        let json = serde_json::to_vec(&meta)?;
        sqlx::query("INSERT OR REPLACE INTO metadata_cache (conn_id, data) VALUES (?, ?)")
            .bind(conn_id)
            .bind(json.as_slice())
            .execute(&self.pool)
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

        use sqlx::Row as _;
        let row = sqlx::query("SELECT data FROM metadata_cache WHERE conn_id = ?")
            .bind(conn_id)
            .fetch_optional(&self.pool)
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
    pub async fn preload_from_disk(&self) -> Result<()> {
        use sqlx::Row as _;
        let rows = sqlx::query("SELECT conn_id, data FROM metadata_cache")
            .fetch_all(&self.pool)
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

    async fn open_memory() -> MetadataCache {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        MetadataCache::new(pool).await.unwrap()
    }

    async fn open_at(path: &std::path::Path) -> MetadataCache {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(path)
                    .create_if_missing(true),
            )
            .await
            .unwrap();
        MetadataCache::new(pool).await.unwrap()
    }

    #[tokio::test]
    async fn store_and_load_should_roundtrip_via_file() {
        let dir = tempfile::tempdir().unwrap();
        let cache = open_at(&dir.path().join("wellfeather.db")).await;
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
        let path = dir.path().join("wellfeather.db");

        // Instance A — store
        let cache_a = open_at(&path).await;
        cache_a.store("conn-1", make_meta("orders")).await.unwrap();
        drop(cache_a);

        // Instance B — fresh memory, same file
        let cache_b = open_at(&path).await;
        let loaded = cache_b.load("conn-1").await.unwrap();

        assert_eq!(loaded.tables[0].name, "orders");
    }

    #[tokio::test]
    async fn preload_from_disk_should_populate_memory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wellfeather.db");

        let cache_a = open_at(&path).await;
        cache_a
            .store("conn-1", make_meta("products"))
            .await
            .unwrap();
        drop(cache_a);

        let cache_b = open_at(&path).await;
        cache_b.preload_from_disk().await.unwrap();

        let loaded = cache_b.load("conn-1").await.unwrap();
        assert_eq!(loaded.tables[0].name, "products");
    }

    #[tokio::test]
    async fn load_should_return_none_for_unknown_conn_id() {
        let cache = open_memory().await;
        let result = cache.load("does-not-exist").await;
        assert!(result.is_none());
    }
}

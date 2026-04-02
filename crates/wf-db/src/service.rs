use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::error::DbError;
use crate::models::DbConnection;
use crate::pool::DbPool;

// ---------------------------------------------------------------------------
// DbService
// ---------------------------------------------------------------------------

/// Manages a set of active database connection pools, keyed by `connection_id`.
///
/// `DbService` is cheap to clone — all clones share the same underlying map.
#[derive(Clone, Default)]
pub struct DbService {
    pools: Arc<RwLock<HashMap<String, DbPool>>>,
}

impl DbService {
    /// Create a new, empty `DbService`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Connect to the database described by `conn` and store the pool.
    ///
    /// `password` is the **plaintext** password.  The caller (typically
    /// `AppController`) is responsible for decrypting `conn.password_encrypted`
    /// via `wf-config::crypto` before calling this method.
    ///
    /// In connection-string mode the password is embedded in the URL and
    /// `password` should be `None`.
    ///
    /// Returns `Ok(())` if the pool was created and stored.
    /// Returns `Err(DbError::ConnectionFailed)` if the underlying connection fails.
    pub async fn connect(
        &self,
        conn: &DbConnection,
        password: Option<&str>,
    ) -> Result<(), DbError> {
        let pool = DbPool::connect(conn, password).await?;
        self.pools
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .insert(conn.id.clone(), pool);
        Ok(())
    }

    /// Disconnect from the database identified by `conn_id`.
    ///
    /// Removing the pool from the map drops it, which closes all underlying
    /// connections held by the pool. If `conn_id` is not found, this is a no-op.
    pub fn disconnect(&self, conn_id: &str) {
        self.pools
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .remove(conn_id);
    }

    /// Returns `true` if a pool for `conn_id` exists in the map.
    pub fn is_connected(&self, conn_id: &str) -> bool {
        self.pools
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .contains_key(conn_id)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::DbType;

    fn sqlite_memory_conn(id: &str) -> DbConnection {
        DbConnection {
            id: id.to_string(),
            name: id.to_string(),
            db_type: DbType::SQLite,
            connection_string: Some("sqlite::memory:".to_string()),
            host: None,
            port: None,
            user: None,
            password_encrypted: None,
            database: None,
        }
    }

    #[tokio::test]
    async fn connect_should_add_pool_to_map() {
        let svc = DbService::new();
        let conn = sqlite_memory_conn("conn-1");

        assert!(!svc.is_connected("conn-1"));
        svc.connect(&conn, None).await.unwrap();
        assert!(svc.is_connected("conn-1"));
    }

    #[tokio::test]
    async fn disconnect_should_remove_pool_from_map() {
        let svc = DbService::new();
        let conn = sqlite_memory_conn("conn-2");

        svc.connect(&conn, None).await.unwrap();
        assert!(svc.is_connected("conn-2"));

        svc.disconnect("conn-2");
        assert!(!svc.is_connected("conn-2"));
    }

    #[tokio::test]
    async fn disconnect_on_unknown_id_should_be_noop() {
        let svc = DbService::new();
        // should not panic
        svc.disconnect("nonexistent");
        assert!(!svc.is_connected("nonexistent"));
    }

    #[tokio::test]
    async fn connect_multiple_should_track_independently() {
        let svc = DbService::new();
        let c1 = sqlite_memory_conn("a");
        let c2 = sqlite_memory_conn("b");

        svc.connect(&c1, None).await.unwrap();
        svc.connect(&c2, None).await.unwrap();

        assert!(svc.is_connected("a"));
        assert!(svc.is_connected("b"));

        svc.disconnect("a");
        assert!(!svc.is_connected("a"));
        assert!(svc.is_connected("b"));
    }

    #[tokio::test]
    async fn cloned_service_should_share_state() {
        let svc = DbService::new();
        // clone required: verify Arc sharing between two handles
        let svc2 = svc.clone();

        let conn = sqlite_memory_conn("shared");
        svc.connect(&conn, None).await.unwrap();

        assert!(svc2.is_connected("shared"));
    }
}

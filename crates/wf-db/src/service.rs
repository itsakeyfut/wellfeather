use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use tokio_util::sync::CancellationToken;

use crate::error::DbError;
use crate::models::{DbConnection, DbMetadata, QueryResult};
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

    /// Execute `sql` on the connection identified by `conn_id`.
    ///
    /// Returns `Err(DbError::ConnectionFailed)` if `conn_id` is not in the pool map.
    pub async fn execute(&self, conn_id: &str, sql: &str) -> Result<QueryResult, DbError> {
        let pool = self.pool_for(conn_id)?;
        pool.execute(sql).await
    }

    /// Execute `sql` on the connection identified by `conn_id`, aborting if
    /// `token` is cancelled before the query completes.
    ///
    /// Returns `Err(DbError::Cancelled)` if the token fires first.
    /// Returns `Err(DbError::ConnectionFailed)` if `conn_id` is not in the pool map.
    pub async fn execute_with_cancel(
        &self,
        conn_id: &str,
        sql: &str,
        token: CancellationToken,
    ) -> Result<QueryResult, DbError> {
        let pool = self.pool_for(conn_id)?;
        tokio::select! {
            result = pool.execute(sql) => result,
            _ = token.cancelled() => Err(DbError::Cancelled),
        }
    }

    /// Fetch schema metadata for the connection identified by `conn_id`.
    ///
    /// Returns `Err(DbError::ConnectionFailed)` if `conn_id` is not connected.
    pub async fn fetch_metadata(&self, conn_id: &str) -> Result<DbMetadata, DbError> {
        let pool = self.pool_for(conn_id)?;
        pool.fetch_metadata().await
    }

    /// Returns `true` if a pool for `conn_id` exists in the map.
    pub fn is_connected(&self, conn_id: &str) -> bool {
        self.pools
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .contains_key(conn_id)
    }

    // ── private helpers ───────────────────────────────────────────────────────

    /// Clone the [`DbPool`] for `conn_id` out of the read-lock so it can be
    /// used across `.await` points without holding the lock.
    fn pool_for(&self, conn_id: &str) -> Result<DbPool, DbError> {
        self.pools
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .get(conn_id)
            .cloned()
            .ok_or_else(|| DbError::ConnectionFailed(format!("not connected: {conn_id}")))
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

    // ── execute ───────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn execute_should_return_query_result_for_connected_id() {
        let svc = DbService::new();
        let conn = sqlite_memory_conn("exec-1");
        svc.connect(&conn, None).await.unwrap();

        let result = svc.execute("exec-1", "SELECT 42 AS answer").await.unwrap();

        assert_eq!(result.row_count, 1);
        assert_eq!(result.rows[0][0], Some("42".to_string()));
    }

    #[tokio::test]
    async fn execute_should_return_connection_failed_for_unknown_id() {
        let svc = DbService::new();

        let err = svc.execute("unknown", "SELECT 1").await.unwrap_err();

        assert!(matches!(err, DbError::ConnectionFailed(_)));
    }

    // ── execute_with_cancel ───────────────────────────────────────────────────

    #[tokio::test]
    async fn execute_with_cancel_should_return_result_when_not_cancelled() {
        let svc = DbService::new();
        let conn = sqlite_memory_conn("cancel-1");
        svc.connect(&conn, None).await.unwrap();

        let token = CancellationToken::new();
        let result = svc
            .execute_with_cancel("cancel-1", "SELECT 1", token)
            .await
            .unwrap();

        assert_eq!(result.row_count, 1);
    }

    #[tokio::test]
    async fn execute_with_cancel_should_return_cancelled_when_token_fires() {
        let svc = DbService::new();
        let conn = sqlite_memory_conn("cancel-2");
        svc.connect(&conn, None).await.unwrap();

        let token = CancellationToken::new();
        token.cancel(); // fire immediately before the query starts

        let err = svc
            .execute_with_cancel("cancel-2", "SELECT 1", token)
            .await
            .unwrap_err();

        assert!(matches!(err, DbError::Cancelled));
    }

    #[tokio::test]
    async fn execute_with_cancel_should_return_connection_failed_for_unknown_id() {
        let svc = DbService::new();
        let token = CancellationToken::new();

        let err = svc
            .execute_with_cancel("unknown", "SELECT 1", token)
            .await
            .unwrap_err();

        assert!(matches!(err, DbError::ConnectionFailed(_)));
    }
}

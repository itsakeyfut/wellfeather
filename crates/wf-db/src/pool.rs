use sqlx::{MySqlPool, PgPool, SqlitePool};

use crate::drivers;
use crate::error::DbError;
use crate::models::{DbConnection, DbKind, DbMetadata, DbType, QueryResult};

// ---------------------------------------------------------------------------
// DbPool
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub enum DbPool {
    Pg(PgPool),
    My(MySqlPool),
    Sqlite(SqlitePool),
}

impl DbPool {
    /// Connect to the database described by `conn`.
    ///
    /// For individual-field mode, the caller must supply the plaintext
    /// `password` (after decrypting `conn.password_encrypted` via
    /// `wf-config::crypto`).  In connection-string mode the password is
    /// already embedded in the URL and `password` is ignored.
    pub async fn connect(conn: &DbConnection, password: Option<&str>) -> Result<Self, DbError> {
        match conn.db_type {
            DbType::PostgreSQL => {
                let url = pg_url(conn, password);
                let pool = PgPool::connect(&url)
                    .await
                    .map_err(|e| DbError::ConnectionFailed(e.to_string()))?;
                Ok(DbPool::Pg(pool))
            }
            DbType::MySQL => {
                let url = my_url(conn, password);
                let pool = MySqlPool::connect(&url)
                    .await
                    .map_err(|e| DbError::ConnectionFailed(e.to_string()))?;
                Ok(DbPool::My(pool))
            }
            DbType::SQLite => {
                let url = sqlite_url(conn);
                let pool = SqlitePool::connect(&url)
                    .await
                    .map_err(|e| DbError::ConnectionFailed(e.to_string()))?;
                Ok(DbPool::Sqlite(pool))
            }
        }
    }

    /// Execute `sql` against this pool and return a [`QueryResult`].
    ///
    /// Dispatches to the correct driver (`sqlite`, `pg`, or `my`) based on
    /// the pool variant.
    pub async fn execute(&self, sql: &str) -> Result<QueryResult, DbError> {
        match self {
            DbPool::Pg(p) => drivers::pg::execute(p, sql).await,
            DbPool::My(p) => drivers::my::execute(p, sql).await,
            DbPool::Sqlite(p) => drivers::sqlite::execute(p, sql).await,
        }
    }

    /// Fetch schema metadata (tables, views, stored procs, indexes) for this pool.
    pub async fn fetch_metadata(&self) -> Result<DbMetadata, DbError> {
        match self {
            DbPool::Pg(p) => drivers::pg::fetch_metadata(p).await,
            DbPool::My(p) => drivers::my::fetch_metadata(p).await,
            DbPool::Sqlite(p) => drivers::sqlite::fetch_metadata(p).await,
        }
    }

    /// Returns the [`DbKind`] variant that identifies which DB engine this pool targets.
    pub fn kind(&self) -> DbKind {
        match self {
            DbPool::Pg(_) => DbKind::Postgres,
            DbPool::My(_) => DbKind::MySql,
            DbPool::Sqlite(_) => DbKind::Sqlite,
        }
    }
}

// ---------------------------------------------------------------------------
// URL helpers (pub(crate) for unit-test visibility)
// ---------------------------------------------------------------------------

/// Build a PostgreSQL connection URL from `conn` and a plaintext `password`.
/// Returns `conn.connection_string` unchanged if present (string-mode takes priority).
pub(crate) fn pg_url(conn: &DbConnection, password: Option<&str>) -> String {
    if let Some(url) = &conn.connection_string {
        return url.clone();
    }
    let host = conn.host.as_deref().unwrap_or("localhost");
    let port = conn.port.unwrap_or(5432);
    let user = conn.user.as_deref().unwrap_or("");
    let db = conn.database.as_deref().unwrap_or("");
    match password {
        Some(pw) if !pw.is_empty() => {
            format!("postgresql://{}:{}@{}:{}/{}", user, pw, host, port, db)
        }
        _ => format!("postgresql://{}@{}:{}/{}", user, host, port, db),
    }
}

/// Build a MySQL connection URL from `conn` and a plaintext `password`.
/// Returns `conn.connection_string` unchanged if present.
pub(crate) fn my_url(conn: &DbConnection, password: Option<&str>) -> String {
    if let Some(url) = &conn.connection_string {
        return url.clone();
    }
    let host = conn.host.as_deref().unwrap_or("localhost");
    let port = conn.port.unwrap_or(3306);
    let user = conn.user.as_deref().unwrap_or("");
    let db = conn.database.as_deref().unwrap_or("");
    match password {
        Some(pw) if !pw.is_empty() => {
            format!("mysql://{}:{}@{}:{}/{}", user, pw, host, port, db)
        }
        _ => format!("mysql://{}@{}:{}/{}", user, host, port, db),
    }
}

/// Build a SQLite connection URL from `conn`.
/// Returns `conn.connection_string` unchanged if present.
/// Treats `":memory:"` and `None` as an in-process SQLite database.
pub(crate) fn sqlite_url(conn: &DbConnection) -> String {
    if let Some(url) = &conn.connection_string {
        return url.clone();
    }
    match conn.database.as_deref() {
        Some(":memory:") | None => "sqlite::memory:".to_string(),
        Some(path) => format!("sqlite:{}", path),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{DbConnection, DbType};

    fn pg_conn_fields() -> DbConnection {
        DbConnection {
            id: "pg-test".to_string(),
            name: "pg-test".to_string(),
            db_type: DbType::PostgreSQL,
            connection_string: None,
            host: Some("db.example.com".to_string()),
            port: Some(5432),
            user: Some("alice".to_string()),
            password_encrypted: None,
            database: Some("mydb".to_string()),
        }
    }

    fn my_conn_fields() -> DbConnection {
        DbConnection {
            id: "my-test".to_string(),
            name: "my-test".to_string(),
            db_type: DbType::MySQL,
            connection_string: None,
            host: Some("mysql.example.com".to_string()),
            port: Some(3306),
            user: Some("bob".to_string()),
            password_encrypted: None,
            database: Some("shop".to_string()),
        }
    }

    fn sqlite_conn_memory() -> DbConnection {
        DbConnection {
            id: "sqlite-mem".to_string(),
            name: "sqlite-mem".to_string(),
            db_type: DbType::SQLite,
            connection_string: Some("sqlite::memory:".to_string()),
            host: None,
            port: None,
            user: None,
            password_encrypted: None,
            database: None,
        }
    }

    fn sqlite_conn_fields_memory() -> DbConnection {
        DbConnection {
            id: "sqlite-mem-fields".to_string(),
            name: "sqlite-mem-fields".to_string(),
            db_type: DbType::SQLite,
            connection_string: None,
            host: None,
            port: None,
            user: None,
            password_encrypted: None,
            database: None, // None → :memory:
        }
    }

    // -- URL helper tests (synchronous) ------------------------------------

    #[test]
    fn pg_url_should_build_from_fields() {
        let conn = pg_conn_fields();
        let url = pg_url(&conn, Some("s3cr3t"));
        assert_eq!(url, "postgresql://alice:s3cr3t@db.example.com:5432/mydb");
    }

    #[test]
    fn pg_url_should_omit_password_when_none() {
        let conn = pg_conn_fields();
        let url = pg_url(&conn, None);
        assert_eq!(url, "postgresql://alice@db.example.com:5432/mydb");
    }

    #[test]
    fn pg_url_should_use_connection_string_when_present() {
        let mut conn = pg_conn_fields();
        conn.connection_string = Some("postgresql://override:5555/other".to_string());
        let url = pg_url(&conn, Some("ignored"));
        assert_eq!(url, "postgresql://override:5555/other");
    }

    #[test]
    fn my_url_should_build_from_fields() {
        let conn = my_conn_fields();
        let url = my_url(&conn, Some("pass123"));
        assert_eq!(url, "mysql://bob:pass123@mysql.example.com:3306/shop");
    }

    #[test]
    fn sqlite_url_should_return_memory_url_when_database_is_none() {
        let conn = sqlite_conn_fields_memory();
        assert_eq!(sqlite_url(&conn), "sqlite::memory:");
    }

    #[test]
    fn sqlite_url_should_use_database_field_as_path() {
        let mut conn = sqlite_conn_fields_memory();
        conn.database = Some("mydb.sqlite".to_string());
        assert_eq!(sqlite_url(&conn), "sqlite:mydb.sqlite");
    }

    // -- Integration tests (require SQLite runtime) ------------------------

    #[tokio::test]
    async fn db_pool_should_connect_sqlite_memory_via_connection_string() {
        let conn = sqlite_conn_memory();
        let pool = DbPool::connect(&conn, None).await.unwrap();
        assert_eq!(pool.kind(), DbKind::Sqlite);
    }

    #[tokio::test]
    async fn db_pool_should_connect_sqlite_memory_via_field_mode() {
        let conn = sqlite_conn_fields_memory();
        let pool = DbPool::connect(&conn, None).await.unwrap();
        assert_eq!(pool.kind(), DbKind::Sqlite);
    }

    #[tokio::test]
    async fn db_pool_kind_should_return_sqlite_for_sqlite_pool() {
        let conn = sqlite_conn_memory();
        let pool = DbPool::connect(&conn, None).await.unwrap();
        assert_eq!(pool.kind(), DbKind::Sqlite);
    }
}

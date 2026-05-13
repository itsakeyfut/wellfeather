use std::str::FromStr as _;

use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use sqlx::mysql::{MySqlConnectOptions, MySqlSslMode};
use sqlx::postgres::{PgConnectOptions, PgSslMode};
use sqlx::{MySqlPool, PgPool, SqlitePool};

use crate::drivers;
use crate::error::DbError;
#[cfg(test)]
use crate::models::DbKind;
use crate::models::{DbConnection, DbMetadata, DbType, QueryResult, SslMode};

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
                let pool = if let Some(ssl) = &conn.ssl {
                    let pg_ssl = match ssl.mode {
                        SslMode::Require => PgSslMode::Require,
                        SslMode::VerifyCa => PgSslMode::VerifyCa,
                        SslMode::VerifyFull => PgSslMode::VerifyFull,
                    };
                    let mut opts = PgConnectOptions::from_str(&url)
                        .map_err(|e| DbError::SslError(e.to_string()))?
                        .ssl_mode(pg_ssl);
                    if let Some(p) = &ssl.ca_cert {
                        opts = opts.ssl_root_cert(p);
                    }
                    if let Some(p) = &ssl.client_cert {
                        opts = opts.ssl_client_cert(p);
                    }
                    if let Some(p) = &ssl.client_key {
                        opts = opts.ssl_client_key(p);
                    }
                    PgPool::connect_with(opts).await.map_err(|e| {
                        DbError::ConnectionFailed(redact_url_password(&e.to_string()))
                    })?
                } else {
                    PgPool::connect(&url).await.map_err(|e| {
                        DbError::ConnectionFailed(redact_url_password(&e.to_string()))
                    })?
                };
                Ok(DbPool::Pg(pool))
            }
            DbType::MySQL => {
                let url = my_url(conn, password);
                let pool = if let Some(ssl) = &conn.ssl {
                    let my_ssl = match ssl.mode {
                        SslMode::Require => MySqlSslMode::Required,
                        SslMode::VerifyCa => MySqlSslMode::VerifyCa,
                        SslMode::VerifyFull => MySqlSslMode::VerifyIdentity,
                    };
                    let mut opts = MySqlConnectOptions::from_str(&url)
                        .map_err(|e| DbError::SslError(e.to_string()))?
                        .ssl_mode(my_ssl);
                    if let Some(p) = &ssl.ca_cert {
                        opts = opts.ssl_ca(p);
                    }
                    if let Some(p) = &ssl.client_cert {
                        opts = opts.ssl_client_cert(p);
                    }
                    if let Some(p) = &ssl.client_key {
                        opts = opts.ssl_client_key(p);
                    }
                    MySqlPool::connect_with(opts).await.map_err(|e| {
                        DbError::ConnectionFailed(redact_url_password(&e.to_string()))
                    })?
                } else {
                    MySqlPool::connect(&url).await.map_err(|e| {
                        DbError::ConnectionFailed(redact_url_password(&e.to_string()))
                    })?
                };
                Ok(DbPool::My(pool))
            }
            DbType::SQLite => {
                let url = sqlite_url(conn)?;
                let pool = SqlitePool::connect(&url)
                    .await
                    .map_err(|e| DbError::ConnectionFailed(redact_url_password(&e.to_string())))?;
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

    /// Fetch the DDL `CREATE` statement for the object named `name`.
    ///
    /// `kind` should be `"table"`, `"view"`, or `"index"`.
    pub async fn fetch_ddl(&self, name: &str, kind: &str) -> Result<String, DbError> {
        match self {
            DbPool::Pg(p) => drivers::pg::fetch_ddl(p, name, kind).await,
            DbPool::My(p) => drivers::my::fetch_ddl(p, name, kind).await,
            DbPool::Sqlite(p) => drivers::sqlite::fetch_ddl(p, name, kind).await,
        }
    }

    /// Returns the [`DbKind`] variant that identifies which DB engine this pool targets.
    #[cfg(test)]
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

/// Percent-encode a credential component (username or password) for safe
/// embedding in a connection URL authority section.
fn encode_credential(s: &str) -> String {
    utf8_percent_encode(s, NON_ALPHANUMERIC).to_string()
}

/// Build a PostgreSQL connection URL from `conn` and a plaintext `password`.
/// Returns `conn.connection_string` unchanged if present (string-mode takes priority).
/// Username and password are percent-encoded so reserved characters (`@`, `:`,
/// `/`, etc.) do not corrupt the URL.
pub(crate) fn pg_url(conn: &DbConnection, password: Option<&str>) -> String {
    if let Some(url) = &conn.connection_string {
        return url.clone();
    }
    let host = conn.host.as_deref().unwrap_or("localhost");
    let port = conn.port.unwrap_or(5432);
    let user = encode_credential(conn.user.as_deref().unwrap_or(""));
    let db = conn.database.as_deref().unwrap_or("");
    match password {
        Some(pw) if !pw.is_empty() => {
            format!(
                "postgresql://{}:{}@{}:{}/{}",
                user,
                encode_credential(pw),
                host,
                port,
                db
            )
        }
        _ => format!("postgresql://{}@{}:{}/{}", user, host, port, db),
    }
}

/// Build a MySQL connection URL from `conn` and a plaintext `password`.
/// Returns `conn.connection_string` unchanged if present.
/// Username and password are percent-encoded so reserved characters do not
/// corrupt the URL.
pub(crate) fn my_url(conn: &DbConnection, password: Option<&str>) -> String {
    if let Some(url) = &conn.connection_string {
        return url.clone();
    }
    let host = conn.host.as_deref().unwrap_or("localhost");
    let port = conn.port.unwrap_or(3306);
    let user = encode_credential(conn.user.as_deref().unwrap_or(""));
    let db = conn.database.as_deref().unwrap_or("");
    match password {
        Some(pw) if !pw.is_empty() => {
            format!(
                "mysql://{}:{}@{}:{}/{}",
                user,
                encode_credential(pw),
                host,
                port,
                db
            )
        }
        _ => format!("mysql://{}@{}:{}/{}", user, host, port, db),
    }
}

/// Build a SQLite connection URL from `conn`.
/// Returns `conn.connection_string` unchanged if present.
/// `None` database maps to an in-process SQLite database.
///
/// Rejects paths that start with `:` or `file:` (special SQLite URI forms)
/// and paths that contain `..` components (path traversal).
pub(crate) fn sqlite_url(conn: &DbConnection) -> Result<String, DbError> {
    if let Some(url) = &conn.connection_string {
        return Ok(url.clone());
    }
    match conn.database.as_deref() {
        None => Ok("sqlite::memory:".to_string()),
        Some(path) => {
            if path.starts_with(':') || path.starts_with("file:") {
                return Err(DbError::InvalidConfig(
                    "SQLite path must be a file path, not a special URI".into(),
                ));
            }
            if path.split(['/', '\\']).any(|c| c == "..") {
                return Err(DbError::InvalidConfig(
                    "SQLite path must not contain '..'".into(),
                ));
            }
            Ok(format!("sqlite:{}", path))
        }
    }
}

// ---------------------------------------------------------------------------
// URL password redaction
// ---------------------------------------------------------------------------

/// Replace the password component in any `scheme://user:password@host` URLs
/// found in `s` with `***`.  Handles multiple URLs in one string.
/// URLs without a password (no `:pw@` in the authority) are left unchanged.
pub(crate) fn redact_url_password(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(scheme_end) = rest.find("://") {
        // copy everything up to and including "://"
        result.push_str(&rest[..scheme_end + 3]);
        rest = &rest[scheme_end + 3..];

        // authority ends at the first '/' or end-of-string
        let authority_len = rest.find('/').unwrap_or(rest.len());
        let authority = &rest[..authority_len];

        if let Some(at_pos) = authority.find('@') {
            let user_info = &authority[..at_pos];
            if let Some(colon_pos) = user_info.find(':') {
                // user_info has a password — redact it
                result.push_str(&user_info[..colon_pos + 1]); // "user:"
                result.push_str("***");
                result.push_str(&authority[at_pos..]); // "@host:port/..."
                rest = &rest[authority_len..];
                continue;
            }
        }
        // No password found — copy authority verbatim
        result.push_str(authority);
        rest = &rest[authority_len..];
    }
    result.push_str(rest);
    result
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{DbConnection, DbType, SslMode};

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
            ssh: None,
            ssl: None,
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
            ssh: None,
            ssl: None,
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
            ssh: None,
            ssl: None,
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
            ssh: None,
            ssl: None,
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
    fn pg_url_should_percent_encode_special_chars_in_password() {
        let conn = pg_conn_fields();
        // "@" in password would break URL authority parsing if not encoded
        let url = pg_url(&conn, Some("pass@word"));
        assert!(
            !url.contains("pass@word"),
            "raw @ must not appear in URL: {url}"
        );
        assert!(
            url.contains("pass%40word"),
            "@ must be percent-encoded as %40: {url}"
        );
        // ":" in password would split user:pass at wrong byte
        let url2 = pg_url(&conn, Some("pa:ss"));
        assert!(
            !url2.contains(":pa:"),
            "raw : must not appear twice: {url2}"
        );
        assert!(
            url2.contains("pa%3Ass"),
            "colon must be encoded as %3A: {url2}"
        );
    }

    #[test]
    fn my_url_should_percent_encode_special_chars_in_password() {
        let conn = my_conn_fields();
        let url = my_url(&conn, Some("pass@word"));
        assert!(
            !url.contains("pass@word"),
            "raw @ must not appear in URL: {url}"
        );
        assert!(
            url.contains("pass%40word"),
            "@ must be percent-encoded as %40: {url}"
        );
        let url2 = my_url(&conn, Some("p/q?r#s"));
        assert!(
            !url2.contains("p/q"),
            "slash must not appear unencoded: {url2}"
        );
    }

    #[test]
    fn pg_url_should_percent_encode_special_chars_in_username() {
        let mut conn = pg_conn_fields();
        conn.user = Some("al ice".to_string()); // space in username
        let url = pg_url(&conn, None);
        assert!(
            !url.contains("al ice"),
            "raw space must not appear in URL: {url}"
        );
        assert!(url.contains("al%20ice"), "space must be %20-encoded: {url}");
    }

    #[test]
    fn sqlite_url_should_return_memory_url_when_database_is_none() {
        let conn = sqlite_conn_fields_memory();
        assert_eq!(sqlite_url(&conn).unwrap(), "sqlite::memory:");
    }

    #[test]
    fn sqlite_url_should_use_database_field_as_path() {
        let mut conn = sqlite_conn_fields_memory();
        conn.database = Some("mydb.sqlite".to_string());
        assert_eq!(sqlite_url(&conn).unwrap(), "sqlite:mydb.sqlite");
    }

    #[test]
    fn sqlite_url_should_reject_parent_dir_traversal() {
        let mut conn = sqlite_conn_fields_memory();
        conn.database = Some("../../sensitive.db".to_string());
        assert!(
            matches!(sqlite_url(&conn), Err(DbError::InvalidConfig(_))),
            "expected InvalidConfig for path traversal"
        );
    }

    #[test]
    fn sqlite_url_should_reject_colon_prefix() {
        let mut conn = sqlite_conn_fields_memory();
        conn.database = Some(":memory:".to_string());
        assert!(
            matches!(sqlite_url(&conn), Err(DbError::InvalidConfig(_))),
            "expected InvalidConfig for :memory: prefix"
        );
    }

    #[test]
    fn sqlite_url_should_reject_file_uri_prefix() {
        let mut conn = sqlite_conn_fields_memory();
        conn.database = Some("file:path.db?mode=memory".to_string());
        assert!(
            matches!(sqlite_url(&conn), Err(DbError::InvalidConfig(_))),
            "expected InvalidConfig for file: prefix"
        );
    }

    #[test]
    fn sqlite_url_should_accept_absolute_path() {
        let mut conn = sqlite_conn_fields_memory();
        conn.database = Some("/home/user/data/mydb.sqlite".to_string());
        assert_eq!(
            sqlite_url(&conn).unwrap(),
            "sqlite:/home/user/data/mydb.sqlite"
        );
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

    // -- SSL helpers ----------------------------------------------------------

    #[test]
    fn ssl_mode_should_map_to_pg_ssl_mode() {
        assert!(matches!(
            map_pg_ssl_mode(SslMode::Require),
            PgSslMode::Require
        ));
        assert!(matches!(
            map_pg_ssl_mode(SslMode::VerifyCa),
            PgSslMode::VerifyCa
        ));
        assert!(matches!(
            map_pg_ssl_mode(SslMode::VerifyFull),
            PgSslMode::VerifyFull
        ));
    }

    #[test]
    fn ssl_mode_should_map_to_mysql_ssl_mode() {
        assert!(matches!(
            map_mysql_ssl_mode(SslMode::Require),
            MySqlSslMode::Required
        ));
        assert!(matches!(
            map_mysql_ssl_mode(SslMode::VerifyCa),
            MySqlSslMode::VerifyCa
        ));
        assert!(matches!(
            map_mysql_ssl_mode(SslMode::VerifyFull),
            MySqlSslMode::VerifyIdentity
        ));
    }

    fn map_pg_ssl_mode(m: SslMode) -> PgSslMode {
        match m {
            SslMode::Require => PgSslMode::Require,
            SslMode::VerifyCa => PgSslMode::VerifyCa,
            SslMode::VerifyFull => PgSslMode::VerifyFull,
        }
    }

    fn map_mysql_ssl_mode(m: SslMode) -> MySqlSslMode {
        match m {
            SslMode::Require => MySqlSslMode::Required,
            SslMode::VerifyCa => MySqlSslMode::VerifyCa,
            SslMode::VerifyFull => MySqlSslMode::VerifyIdentity,
        }
    }

    // -- redact_url_password --------------------------------------------------

    #[test]
    fn redact_url_password_should_replace_password_in_pg_url() {
        let input = "failed: postgresql://alice:s3cret@localhost:5432/db";
        let out = redact_url_password(input);
        assert!(!out.contains("s3cret"), "password still present: {out}");
        assert!(out.contains("***"), "redaction marker missing: {out}");
        assert!(
            out.contains("alice:***@localhost"),
            "user and host lost: {out}"
        );
    }

    #[test]
    fn redact_url_password_should_replace_password_in_mysql_url() {
        let input = "error: mysql://bob:pass123@mysql.host:3306/shop";
        let out = redact_url_password(input);
        assert!(!out.contains("pass123"), "password still present: {out}");
        assert!(out.contains("bob:***@"), "expected bob:***@: {out}");
    }

    #[test]
    fn redact_url_password_should_leave_url_without_password_unchanged() {
        let input = "error: postgresql://alice@localhost:5432/db";
        assert_eq!(redact_url_password(input), input);
    }

    #[test]
    fn redact_url_password_should_leave_plain_text_unchanged() {
        let input = "connection refused: timeout after 5s";
        assert_eq!(redact_url_password(input), input);
    }

    #[test]
    fn redact_url_password_should_handle_multiple_urls_in_one_string() {
        let input = "pg://u:p1@h1/db and mysql://v:p2@h2/db2";
        let out = redact_url_password(input);
        assert!(!out.contains("p1"), "first password still present: {out}");
        assert!(!out.contains("p2"), "second password still present: {out}");
    }
}

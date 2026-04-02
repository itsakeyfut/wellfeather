use sqlx::SqlitePool;

use crate::error::DbError;

/// Connect to a SQLite database at `url`.
///
/// `url` may be:
/// - `"sqlite::memory:"` — in-process in-memory database
/// - `"sqlite:<path>"` — file-backed database
///
/// Any sqlx connection error is wrapped as [`DbError::ConnectionFailed`].
pub async fn connect(url: &str) -> Result<SqlitePool, DbError> {
    SqlitePool::connect(url)
        .await
        .map_err(|e| DbError::ConnectionFailed(e.to_string()))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn connect_should_succeed_with_memory_database() {
        let pool = connect("sqlite::memory:").await.unwrap();
        let row: (i32,) = sqlx::query_as("SELECT 1").fetch_one(&pool).await.unwrap();
        assert_eq!(row.0, 1);
    }

    #[tokio::test]
    async fn connect_should_return_connection_failed_on_invalid_url() {
        let result = connect("postgres://not-sqlite").await;
        assert!(matches!(result, Err(DbError::ConnectionFailed(_))));
    }
}

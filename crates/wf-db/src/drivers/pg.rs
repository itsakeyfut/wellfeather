use sqlx::PgPool;

use crate::error::DbError;

/// Connect to a PostgreSQL database at `url`.
///
/// Any sqlx connection error is wrapped as [`DbError::ConnectionFailed`].
pub async fn connect(url: &str) -> Result<PgPool, DbError> {
    PgPool::connect(url)
        .await
        .map_err(|e| DbError::ConnectionFailed(e.to_string()))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Requires a running PostgreSQL instance.
    /// Set `TEST_PG_URL` to override the default connection string.
    /// Run with: `cargo test -p wf-db -- --ignored`
    #[tokio::test]
    #[ignore]
    async fn connect_should_succeed_with_real_postgres() {
        let url = std::env::var("TEST_PG_URL").unwrap_or_else(|_| {
            "postgresql://postgres:postgres@localhost:5432/postgres".to_string()
        });
        let pool = connect(&url).await.unwrap();
        let row: (i32,) = sqlx::query_as("SELECT 1").fetch_one(&pool).await.unwrap();
        assert_eq!(row.0, 1);
    }

    #[tokio::test]
    #[ignore]
    async fn connect_should_return_connection_failed_on_unreachable_host() {
        let result = connect("postgresql://user:pass@127.0.0.1:19999/db").await;
        assert!(matches!(result, Err(DbError::ConnectionFailed(_))));
    }
}

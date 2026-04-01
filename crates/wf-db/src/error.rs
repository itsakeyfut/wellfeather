/// Typed error for the `wf-db` crate.
///
/// Library-level errors use `thiserror` for typed variants.
/// `AppController` and above use `anyhow` for contextual wrapping.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("connection failed: {0}")]
    ConnectionFailed(String),

    #[error("query execution error: {0}")]
    QueryError(String),

    #[error("query cancelled")]
    Cancelled,

    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_error_cancelled_should_display_correctly() {
        assert_eq!(DbError::Cancelled.to_string(), "query cancelled");
    }

    #[test]
    fn db_error_connection_failed_should_include_message() {
        let e = DbError::ConnectionFailed("timeout".to_string());
        assert_eq!(e.to_string(), "connection failed: timeout");
    }

    #[test]
    fn db_error_query_error_should_include_message() {
        let e = DbError::QueryError("syntax error".to_string());
        assert_eq!(e.to_string(), "query execution error: syntax error");
    }
}

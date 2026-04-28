use rust_i18n::t;
use wf_db::error::DbError;

pub trait LocalizedMessage {
    fn localized_message(&self) -> String;
}

impl LocalizedMessage for DbError {
    fn localized_message(&self) -> String {
        match self {
            DbError::ConnectionFailed(s) => t!("error.db_connect_failed", reason = s).to_string(),
            DbError::QueryError(s) => t!("error.query_failed", reason = s).to_string(),
            DbError::Cancelled => t!("error.query_cancelled").to_string(),
            DbError::Sqlx(e) => t!("error.db_error", reason = e.to_string()).to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use rust_i18n::t;

    #[test]
    fn t_macro_should_resolve_error_db_error_key() {
        let result = t!("error.db_error", reason = "test reason").to_string();
        assert_ne!(
            result, "error.db_error",
            "t!() returned raw key — translations not compiled in"
        );
        assert!(
            result.contains("test reason"),
            "placeholder not interpolated: {result}"
        );
    }

    #[test]
    fn t_macro_should_resolve_status_running_key() {
        let result = t!("status.running").to_string();
        assert_ne!(
            result, "status.running",
            "t!() returned raw key — translations not compiled in"
        );
    }
}

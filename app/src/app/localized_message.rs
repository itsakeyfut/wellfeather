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

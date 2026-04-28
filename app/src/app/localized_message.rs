use crate::app::locale;
use wf_db::error::DbError;

pub trait LocalizedMessage {
    fn localized_message(&self) -> String;
}

impl LocalizedMessage for DbError {
    fn localized_message(&self) -> String {
        match self {
            DbError::ConnectionFailed(s) => {
                locale::tr("error.db_connect_failed", &[("reason", s.as_str())])
            }
            DbError::QueryError(s) => locale::tr("error.query_failed", &[("reason", s.as_str())]),
            DbError::Cancelled => locale::tr("error.query_cancelled", &[]),
            DbError::Sqlx(e) => {
                let reason = e.to_string();
                locale::tr("error.db_error", &[("reason", &reason)])
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::app::locale;

    #[test]
    fn tr_should_resolve_error_db_error_key() {
        locale::set_locale("en");
        let result = locale::tr("error.db_error", &[("reason", "test reason")]);
        assert_ne!(result, "error.db_error", "tr() returned raw key");
        assert!(
            result.contains("test reason"),
            "placeholder not interpolated: {result}"
        );
    }

    #[test]
    fn tr_should_resolve_status_running_key() {
        locale::set_locale("en");
        let result = locale::tr("status.running", &[]);
        assert_ne!(result, "status.running", "tr() returned raw key");
    }
}

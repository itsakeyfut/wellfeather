/// Typed error for [`crate::service::HistoryService`] operations.
#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

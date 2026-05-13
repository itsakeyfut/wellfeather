/// Typed error for [`super::GroupRepository`] operations.
#[derive(Debug, thiserror::Error)]
pub enum RepositoryError {
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

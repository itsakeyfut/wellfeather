use wf_completion::CompletionItem;
use wf_config::models::Theme;
use wf_db::models::{DbMetadata, QueryResult};

/// Fine-grained state transitions forwarded to the UI via `StateChanged`.
#[derive(Debug)]
pub enum StateEvent {
    QueryStarted,
    QueryFinished(QueryResult),
    ConnectionChanged(String),
    ThemeChanged(Theme),
    LoadingChanged(bool),
}

/// Controller → UI channel messages.
#[derive(Debug)]
pub enum Event {
    Connected(String), // connection_id
    Disconnected(String),
    QueryStarted,
    QueryFinished(QueryResult),
    QueryCancelled,
    QueryError(String),
    CompletionReady(Vec<CompletionItem>),
    MetadataLoaded(DbMetadata),
    ConfigUpdated,
    StateChanged(StateEvent),
}

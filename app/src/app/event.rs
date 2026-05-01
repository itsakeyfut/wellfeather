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
    /// Fired when `Command::Connect` fails. Distinct from `QueryError` so the UI
    /// can display the message in the status bar / form without touching the result area.
    ConnectError(String),
    QueryStarted,
    QueryFinished(QueryResult),
    QueryCancelled,
    QueryError(String),
    /// Fired when `Command::TestConnection` succeeds (connection verified, then dropped).
    TestConnectionOk,
    /// Fired when `Command::TestConnection` fails; carries the error message.
    TestConnectionFailed(String),
    CompletionReady(Vec<CompletionItem>),
    MetadataLoaded(String, DbMetadata), // conn_id, metadata
    MetadataFetchFailed(String),
    /// Insert text into the SQL editor (append after existing content).
    InsertText(String),
    ConfigUpdated,
    StateChanged(StateEvent),
    DdlLoaded {
        tab_id: String,
        ddl: String,
    },
    DdlFetchFailed {
        tab_id: String,
        msg: String,
    },
    TableDataLoaded {
        tab_id: String,
        result: QueryResult,
    },
    TableDataFailed {
        tab_id: String,
        msg: String,
    },
}

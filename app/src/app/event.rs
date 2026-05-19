use tokio::sync::oneshot;
use wf_completion::CompletionItem;
use wf_config::models::{ConnectionConfig, GroupConfig, Theme};
use wf_db::models::{DbMetadata, QueryExecution, QueryResult};

/// Fine-grained state transitions forwarded to the UI via `StateChanged`.
#[derive(Debug)]
pub enum StateEvent {
    ThemeChanged(Theme),
}

/// Controller → UI channel messages.
#[derive(Debug)]
pub enum Event {
    Connected {
        id: String,
        /// Full list of all saved connections at connect time.
        connections: Vec<ConnectionConfig>,
        safe_dml: bool,
        read_only: bool,
    },
    Disconnected(String),
    ConnectionRemoved(String), // connection_id — deleted from config
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
    ConfigUpdated,
    /// Fired after `ConfigUpdate::ConnectionFlags` is persisted.
    /// Carries the connection id and the new `read_only` value so the sidebar
    /// can update the lock icon without a full reconnect.
    ConnectionFlagsUpdated {
        id: String,
        read_only: bool,
        safe_dml: bool,
    },
    /// Controller paused a connect attempt awaiting host-key approval.
    /// The UI shows a fingerprint dialog; sending `true` to `approval_tx` proceeds,
    /// `false` aborts the connection.
    SshFingerprintRequired {
        fingerprint: String,
        approval_tx: oneshot::Sender<bool>,
    },
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
    // ── History panel ────────────────────────────────────────────────────────
    HistoryLoaded(Vec<QueryExecution>),
    // ── Group events ─────────────────────────────────────────────────────────
    /// A new group was created; `id` is the new group's ID. Used by the UI to
    /// trigger inline rename on the newly added group node.
    GroupCreated {
        id: String,
        groups: Vec<GroupConfig>,
        connections: Vec<ConnectionConfig>,
    },
    /// Groups or connection group assignments changed; UI should rebuild the sidebar.
    GroupsUpdated {
        groups: Vec<GroupConfig>,
        connections: Vec<ConnectionConfig>,
    },
}

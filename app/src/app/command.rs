use std::path::PathBuf;

use wf_config::models::{ConnectionConfig, PageSize, Theme};
use wf_db::models::DbConnection;

/// Format for result export.
#[derive(Debug)]
pub enum ExportFormat {
    Csv,
    Json,
}

/// Granular config change sent from the UI.
#[derive(Debug)]
pub enum ConfigUpdate {
    Theme(Theme),
    PageSize(PageSize),
    FontFamily(String),
    FontSize(u32),
    Language(String),
    Connection(ConnectionConfig),
}

/// UI → Controller channel messages.
#[derive(Debug)]
pub enum Command {
    /// Connect to a database. The second field carries the plaintext password
    /// (decrypted by the caller); `wf-db` must not depend on `wf-config::crypto`.
    /// Password encryption is wired in T028.
    Connect(DbConnection, Option<String>),
    /// Test a connection without persisting it to state or the sidebar.
    /// On success sends [`Event::TestConnectionOk`]; on failure sends
    /// [`Event::TestConnectionFailed`].
    TestConnection(DbConnection, Option<String>),
    Disconnect(String),   // connection_id
    RunQuery(String),     // sql
    RunSelection(String), // sql (selected range)
    RunAll(String),       // sql (entire editor)
    CancelQuery,
    FetchCompletion(String, usize), // sql, cursor_pos
    ExportResult(ExportFormat, PathBuf),
    UpdateConfig(ConfigUpdate),
}

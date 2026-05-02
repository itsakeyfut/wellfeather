use std::path::PathBuf;

use wf_config::models::{PageSize, Theme};
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
    /// Update only safe_dml / read_only flags for an existing connection entry.
    ConnectionFlags {
        id: String,
        safe_dml: bool,
        read_only: bool,
    },
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
    Disconnect(String),       // connection_id
    RemoveConnection(String), // connection_id — disconnect + delete from config
    RunQuery(String),         // sql
    RunSelection(String),     // sql (selected range)
    RunAll(String),           // sql (entire editor)
    CancelQuery,
    FetchCompletion(String, usize), // sql, cursor_pos
    ExportResult(ExportFormat, PathBuf),
    UpdateConfig(ConfigUpdate),
    /// Fetch the DDL CREATE statement for `name` (table/view/index) on `conn_id`.
    FetchDdl {
        tab_id: String,
        conn_id: String,
        name: String,
        kind: String,
    },
    /// Fetch a page of rows from `table_name` on `conn_id` for a Table View tab.
    FetchTableData {
        tab_id: String,
        conn_id: String,
        table_name: String,
        page_size: usize,
    },
}

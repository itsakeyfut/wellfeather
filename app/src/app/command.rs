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
    Connect(DbConnection),
    Disconnect(String),   // connection_id
    RunQuery(String),     // sql
    RunSelection(String), // sql (selected range)
    RunAll(String),       // sql (entire editor)
    CancelQuery,
    FetchCompletion(String, usize), // sql, cursor_pos
    ExportResult(ExportFormat, PathBuf),
    UpdateConfig(ConfigUpdate),
}

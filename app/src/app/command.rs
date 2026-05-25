use wf_config::models::{PageSize, Theme};
use wf_db::models::DbConnection;
use zeroize::Zeroizing;

/// Granular config change sent from the UI.
#[derive(Debug)]
pub enum ConfigUpdate {
    Theme(Theme),
    PageSize(PageSize),
    Language(String),
    /// Update only safe_dml / read_only flags for an existing connection entry.
    ConnectionFlags {
        id: String,
        safe_dml: bool,
        read_only: bool,
    },
    ReduceMotion(bool),
    TabWidth(u32),
    /// Set the query execution timeout (seconds; 0 = disabled).
    QueryTimeout(u64),
    /// Set the slow query warning threshold (milliseconds; 0 = disabled).
    SlowQueryThreshold(u64),
    /// Set the editor font family name.
    FontFamily(String),
    /// Set the editor font size in logical pixels.
    FontSize(u32),
}

/// UI → Controller channel messages.
#[derive(Debug)]
pub enum Command {
    /// Connect to a database. The second field carries the plaintext password
    /// (decrypted by the caller); `wf-db` must not depend on `wf-config::crypto`.
    /// Wrapped in `Zeroizing` so the plaintext is scrubbed from heap on drop.
    Connect(DbConnection, Option<Zeroizing<String>>),
    /// Test a connection without persisting it to state or the sidebar.
    /// On success sends [`Event::TestConnectionOk`]; on failure sends
    /// [`Event::TestConnectionFailed`].
    TestConnection(DbConnection, Option<Zeroizing<String>>),
    Disconnect(String),       // connection_id
    RemoveConnection(String), // connection_id — disconnect + delete from config
    RunQuery(String),         // sql
    RunAll(String),           // sql (entire editor)
    CancelQuery,
    FetchCompletion(String, usize), // sql, cursor_pos
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
    // ── Group management ─────────────────────────────────────────────────────
    /// Create a new connection group with the given name.
    CreateGroup {
        name: String,
    },
    /// Rename an existing group.
    RenameGroup {
        id: String,
        name: String,
    },
    /// Delete a group; connections that belonged to it become ungrouped.
    DeleteGroup {
        id: String,
    },
    /// Move a connection into a group, or ungroup it when `group_id` is `None`.
    MoveConnectionToGroup {
        conn_id: String,
        group_id: Option<String>,
    },
    /// Change the display color of a group (CSS hex string, e.g. "#e74c3c").
    SetGroupColor {
        group_id: String,
        color: String,
    },
    /// Persist the expanded/collapsed state of a group node.
    SetGroupExpanded {
        id: String,
        expanded: bool,
    },
    /// Search query execution history. Empty keyword = most recent N rows.
    SearchHistory {
        keyword: String,
        conn_id: Option<String>,
    },
    /// Undo the last group action (create/rename/color/move/delete).
    UndoGroupAction,
    /// Redo the last undone group action.
    RedoGroupAction,
}

impl Command {
    /// Returns the variant name for logging.  Does **not** include any payload,
    /// so credentials carried by `Connect` and `TestConnection` are never logged.
    pub(crate) fn variant_name(&self) -> &'static str {
        match self {
            Self::Connect(..) => "Connect",
            Self::TestConnection(..) => "TestConnection",
            Self::Disconnect(..) => "Disconnect",
            Self::RemoveConnection(..) => "RemoveConnection",
            Self::RunQuery(..) => "RunQuery",
            Self::RunAll(..) => "RunAll",
            Self::CancelQuery => "CancelQuery",
            Self::FetchCompletion(..) => "FetchCompletion",
            Self::UpdateConfig(..) => "UpdateConfig",
            Self::FetchDdl { .. } => "FetchDdl",
            Self::FetchTableData { .. } => "FetchTableData",
            Self::CreateGroup { .. } => "CreateGroup",
            Self::RenameGroup { .. } => "RenameGroup",
            Self::DeleteGroup { .. } => "DeleteGroup",
            Self::MoveConnectionToGroup { .. } => "MoveConnectionToGroup",
            Self::SetGroupColor { .. } => "SetGroupColor",
            Self::SetGroupExpanded { .. } => "SetGroupExpanded",
            Self::SearchHistory { .. } => "SearchHistory",
            Self::UndoGroupAction => "UndoGroupAction",
            Self::RedoGroupAction => "RedoGroupAction",
        }
    }
}

#[cfg(test)]
mod tests {
    use wf_db::models::{DbConnection, DbType};
    use zeroize::Zeroizing;

    use super::Command;

    fn dummy_conn() -> DbConnection {
        DbConnection {
            id: "test".to_string(),
            name: "test".to_string(),
            db_type: DbType::SQLite,
            connection_string: Some("sqlite::memory:".to_string()),
            host: None,
            port: None,
            user: None,
            password_encrypted: None,
            database: None,
            ssh: None,
            ssl: None,
        }
    }

    #[test]
    fn variant_name_should_return_connect_for_connect_command() {
        let cmd = Command::Connect(dummy_conn(), Some(Zeroizing::new("s3cret".to_string())));
        assert_eq!(cmd.variant_name(), "Connect");
    }

    #[test]
    fn variant_name_should_not_expose_password_in_connect() {
        let cmd = Command::Connect(dummy_conn(), Some(Zeroizing::new("s3cret".to_string())));
        assert!(!cmd.variant_name().contains("s3cret"));
    }

    #[test]
    fn variant_name_should_not_expose_password_in_test_connection() {
        let cmd = Command::TestConnection(dummy_conn(), Some(Zeroizing::new("s3cret".to_string())));
        assert!(!cmd.variant_name().contains("s3cret"));
    }

    #[test]
    fn variant_name_should_return_correct_name_for_each_variant() {
        assert_eq!(
            Command::Disconnect("id".to_string()).variant_name(),
            "Disconnect"
        );
        assert_eq!(
            Command::RunQuery("SELECT 1".to_string()).variant_name(),
            "RunQuery"
        );
        assert_eq!(Command::CancelQuery.variant_name(), "CancelQuery");
    }
}

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

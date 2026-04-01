// ---------------------------------------------------------------------------
// DbType / DbKind
// ---------------------------------------------------------------------------

/// User-visible database type label.
/// Stored in `ConnectionConfig.db_type` and displayed in the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DbType {
    PostgreSQL,
    MySQL,
    SQLite,
}

/// Internal code-level DB variant used for enum dispatch in `DbPool`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DbKind {
    Postgres,
    MySql,
    Sqlite,
}

// ---------------------------------------------------------------------------
// DbConnection
// ---------------------------------------------------------------------------

/// Represents a saved database connection configuration.
///
/// Supports two connection modes (mutually exclusive):
/// - **Connection string**: `postgres://user:pass@host:5432/dbname`
/// - **Individual fields**: host, port, user, password, database
#[derive(Debug, Clone)]
pub struct DbConnection {
    /// UUID identifying this connection.
    pub id: String,
    /// Human-readable display name shown in the sidebar.
    pub name: String,
    pub db_type: DbType,
    /// Connection string mode.
    pub connection_string: Option<String>,
    /// Individual field mode.
    pub host: Option<String>,
    pub port: Option<u16>,
    pub user: Option<String>,
    /// AES-256-GCM encrypted password (see `wf-config::crypto`).
    pub password_encrypted: Option<String>,
    pub database: Option<String>,
}

// ---------------------------------------------------------------------------
// QueryResult
// ---------------------------------------------------------------------------

/// The result of executing a SQL statement.
///
/// `rows` cells use `Option<String>` so that SQL `NULL` values are
/// represented as `None` and are visually distinguishable from empty strings.
#[derive(Debug, Clone, Default)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
    /// Number of rows in `rows` (or affected rows for DML statements).
    pub row_count: usize,
    pub execution_time_ms: u128,
}

// ---------------------------------------------------------------------------
// QueryExecution (history record)
// ---------------------------------------------------------------------------

/// A persisted record of a single query execution, stored in `history.db`.
#[derive(Debug, Clone)]
pub struct QueryExecution {
    /// SQLite ROWID assigned on insert.
    pub id: i64,
    pub sql: String,
    pub duration_ms: u128,
    pub success: bool,
    pub error_message: Option<String>,
    /// Unix epoch seconds.
    pub timestamp: i64,
    pub connection_id: String,
}

// ---------------------------------------------------------------------------
// DbMetadata + helpers
// ---------------------------------------------------------------------------

/// Schema metadata for a single column.
#[derive(Debug, Clone)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
}

/// Schema metadata for a table or view.
#[derive(Debug, Clone)]
pub struct TableInfo {
    pub name: String,
    pub columns: Vec<ColumnInfo>,
}

/// All schema objects retrieved from a connected database.
/// Populated by `DbService::fetch_metadata` and cached in `MetadataCache`.
#[derive(Debug, Clone, Default)]
pub struct DbMetadata {
    pub tables: Vec<TableInfo>,
    pub views: Vec<TableInfo>,
    pub stored_procs: Vec<String>,
    pub indexes: Vec<String>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_result_default_should_be_empty() {
        let r = QueryResult::default();
        assert!(r.columns.is_empty());
        assert!(r.rows.is_empty());
        assert_eq!(r.row_count, 0);
        assert_eq!(r.execution_time_ms, 0);
    }

    #[test]
    fn db_metadata_default_should_have_empty_collections() {
        let m = DbMetadata::default();
        assert!(m.tables.is_empty());
        assert!(m.views.is_empty());
        assert!(m.stored_procs.is_empty());
        assert!(m.indexes.is_empty());
    }
}

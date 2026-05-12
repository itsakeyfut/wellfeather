//! Application controller — the central async command loop.
//!
//! [`AppController`] sits between the UI layer and the service layer. It receives
//! [`Command`] values sent by UI callbacks and translates them into service calls,
//! then broadcasts [`Event`] values back to the UI via `invoke_from_event_loop`.
//!
//! # Communication model
//!
//! ```text
//! UI callbacks  ──(tx_cmd)──▶  AppController::run  ──(tx_event)──▶  UI event handler
//! ```
//!
//! Both channels are bounded (`capacity = CMD_CHANNEL_CAPACITY`). The controller task exits cleanly
//! when all `Sender<Command>` clones are dropped (i.e. when the UI window closes).

mod config;
mod connection;
mod metadata;
mod query;

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::{debug, warn};
use wf_completion::{cache::MetadataCache, service::CompletionService};
use wf_config::ConnectionRepository;
use wf_db::service::DbService;
use wf_history::service::HistoryService;

use crate::{
    app::{command::Command, event::Event, session::SessionManager},
    state::SharedState,
};

const CMD_CHANNEL_CAPACITY: usize = 64;

/// Owns the [`DbService`] pool, the [`SessionManager`] for config persistence,
/// and the [`SharedState`] shared with the UI layer. Created by [`AppController::new`]
/// and consumed by [`AppController::run`], which is spawned as a tokio task.
pub struct AppController {
    state: SharedState,
    db: DbService,
    session: SessionManager,
    repo: Arc<ConnectionRepository>,
    history: HistoryService,
    metadata_cache: MetadataCache,
    completion: CompletionService,
    rx_cmd: mpsc::Receiver<Command>,
    tx_event: mpsc::Sender<Event>,
    /// App config directory — used for the known-hosts file.
    pub(crate) config_dir: PathBuf,
    /// Encryption key for decrypting SSH credentials.
    pub(crate) enc_key: [u8; 32],
}

impl AppController {
    /// Create the controller and return it together with the two channel endpoints
    /// that `main.rs` distributes: `Sender<Command>` → UI, `Receiver<Event>` → UI.
    ///
    /// All services are expected to be fully initialised (schema migrations run)
    /// before being passed in. The shared `SqlitePool` backing them is managed by
    /// the caller (`main.rs`).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        state: SharedState,
        db: DbService,
        session: SessionManager,
        repo: Arc<ConnectionRepository>,
        history: HistoryService,
        metadata_cache: MetadataCache,
        config_dir: PathBuf,
        enc_key: [u8; 32],
    ) -> (Self, mpsc::Sender<Command>, mpsc::Receiver<Event>) {
        let (tx_cmd, rx_cmd) = mpsc::channel(CMD_CHANNEL_CAPACITY);
        let (tx_event, rx_event) = mpsc::channel(CMD_CHANNEL_CAPACITY);
        let completion = CompletionService::new(metadata_cache.clone());
        (
            Self {
                state,
                db,
                session,
                repo,
                history,
                metadata_cache,
                completion,
                rx_cmd,
                tx_event,
                config_dir,
                enc_key,
            },
            tx_cmd,
            rx_event,
        )
    }

    /// Run the command loop as a tokio task (spawn with `tokio::spawn(controller.run())`).
    /// Exits when all `Sender<Command>` clones are dropped.
    pub async fn run(self) {
        if let Err(e) = self.metadata_cache.preload_from_disk().await {
            warn!("failed to preload metadata cache: {e}");
        }

        let mut this = self;
        while let Some(cmd) = this.rx_cmd.recv().await {
            debug!("received command: {}", cmd.variant_name());
            match cmd {
                Command::Connect(conn, pw) => this.handle_connect(conn, pw).await,
                Command::TestConnection(conn, pw) => this.handle_test_connection(conn, pw).await,
                Command::Disconnect(id) => this.handle_disconnect(id).await,
                Command::RemoveConnection(id) => this.handle_remove_connection(id).await,
                Command::RunQuery(sql) => this.handle_run_query(sql).await,
                Command::RunAll(sql) => this.handle_run_all(sql).await,
                Command::CancelQuery => this.handle_cancel_query().await,
                Command::UpdateConfig(update) => this.handle_update_config(update).await,
                Command::FetchCompletion(sql, cursor_pos) => {
                    this.handle_fetch_completion(sql, cursor_pos).await
                }
                Command::FetchDdl {
                    tab_id,
                    conn_id,
                    name,
                    kind,
                } => this.handle_fetch_ddl(tab_id, conn_id, name, kind).await,
                Command::FetchTableData {
                    tab_id,
                    conn_id,
                    table_name,
                    page_size,
                } => {
                    this.handle_fetch_table_data(tab_id, conn_id, table_name, page_size)
                        .await
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Append `LIMIT {limit}` to a SELECT statement that has no explicit LIMIT clause.
///
/// - Non-SELECT statements (INSERT, UPDATE, DELETE, …) are returned unchanged.
/// - Statements that already contain ` LIMIT ` are returned unchanged.
/// - A trailing semicolon is stripped before appending the LIMIT clause.
fn apply_limit(sql: &str, limit: usize) -> String {
    if limit == 0 {
        return sql.to_string();
    }
    let trimmed = sql.trim().trim_end_matches(';').trim_end();
    let upper = trimmed.to_uppercase();
    // Split on whitespace so "\nLIMIT" (formatted SQL) matches as well as " LIMIT ".
    let has_limit = upper.split_whitespace().any(|w| w == "LIMIT");
    if upper.starts_with("SELECT") && !has_limit && !trimmed.contains(';') {
        format!("{} LIMIT {}", trimmed, limit)
    } else {
        sql.to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sqlx::SqlitePool;
    use tempfile::tempdir;
    use wf_completion::cache::MetadataCache;
    use wf_config::{ConnectionRepository, manager::ConfigManager};
    use wf_db::{
        models::{DbConnection, DbType},
        service::DbService,
    };
    use wf_history::service::HistoryService;

    use crate::{
        app::{command::Command, event::Event, session::SessionManager},
        state::AppState,
    };

    use super::{AppController, apply_limit};

    /// Build a [`SessionManager`] backed by a temporary directory.
    /// `keep()` prevents cleanup so the path stays valid for the test lifetime.
    fn test_session() -> SessionManager {
        let dir = tempdir().unwrap();
        let path = dir.keep().join("config.toml");
        SessionManager::with_config_manager(ConfigManager::with_path(path))
    }

    async fn test_repo() -> Arc<ConnectionRepository> {
        Arc::new(ConnectionRepository::open_memory().await.unwrap())
    }

    async fn test_history() -> HistoryService {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        HistoryService::new(pool).await.unwrap()
    }

    async fn test_metadata_cache() -> MetadataCache {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        MetadataCache::new(pool).await.unwrap()
    }

    fn sqlite_conn(id: &str) -> DbConnection {
        DbConnection {
            id: id.to_string(),
            name: id.to_string(),
            db_type: DbType::SQLite,
            connection_string: Some("sqlite::memory:".to_string()),
            host: None,
            port: None,
            user: None,
            password_encrypted: None,
            database: None,
            ssh: None,
        }
    }

    // ── apply_limit ───────────────────────────────────────────────────────────────

    #[test]
    fn apply_limit_should_append_limit_to_select() {
        assert_eq!(
            apply_limit("SELECT * FROM t", 500),
            "SELECT * FROM t LIMIT 500"
        );
    }

    #[test]
    fn apply_limit_should_strip_trailing_semicolon_before_appending() {
        assert_eq!(
            apply_limit("SELECT * FROM t;", 100),
            "SELECT * FROM t LIMIT 100"
        );
    }

    #[test]
    fn apply_limit_should_not_append_when_limit_already_present() {
        let sql = "SELECT * FROM t LIMIT 10";
        assert_eq!(apply_limit(sql, 500), sql);
    }

    #[test]
    fn apply_limit_should_not_modify_dml_statements() {
        let insert = "INSERT INTO t VALUES (1)";
        assert_eq!(apply_limit(insert, 500), insert);
        let update = "UPDATE t SET x = 1";
        assert_eq!(apply_limit(update, 500), update);
        let delete = "DELETE FROM t";
        assert_eq!(apply_limit(delete, 500), delete);
    }

    #[test]
    fn apply_limit_should_be_case_insensitive() {
        assert_eq!(
            apply_limit("select * from t", 1000),
            "select * from t LIMIT 1000"
        );
        let with_limit = "select * from t limit 5";
        assert_eq!(apply_limit(with_limit, 500), with_limit);
    }

    #[test]
    fn apply_limit_should_not_apply_when_limit_is_zero() {
        assert_eq!(apply_limit("SELECT * FROM t", 0), "SELECT * FROM t");
    }

    #[test]
    fn apply_limit_should_not_apply_to_multi_statement_sql() {
        let sql = "SELECT 1; SELECT 2";
        assert_eq!(apply_limit(sql, 500), sql);
    }

    #[test]
    fn apply_limit_should_append_to_formatted_multiline_select() {
        // format_sql("select name from users;") produces this output
        let sql = "SELECT\n  name\nFROM\n  users;";
        assert_eq!(
            apply_limit(sql, 500),
            "SELECT\n  name\nFROM\n  users LIMIT 500"
        );
    }

    #[test]
    fn apply_limit_should_not_duplicate_limit_on_formatted_sql_with_existing_limit() {
        // If the user already wrote LIMIT on its own line, do not append another.
        let sql = "SELECT\n  *\nFROM\n  t\nLIMIT 10";
        assert_eq!(apply_limit(sql, 500), sql);
    }

    // ── TestConnection ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_connection_should_send_ok_and_not_add_to_state() {
        let state = Arc::new(AppState::new());
        let db = DbService::new();
        let (controller, tx_cmd, mut rx_event) = AppController::new(
            state.clone(),
            db,
            test_session(),
            test_repo().await,
            test_history().await,
            test_metadata_cache().await,
            tempdir().unwrap().keep(),
            [0u8; 32],
        );

        tx_cmd
            .send(Command::TestConnection(sqlite_conn("t1"), None))
            .await
            .unwrap();
        drop(tx_cmd);

        controller.run().await;

        let event = rx_event.recv().await.expect("expected event");
        assert!(matches!(event, Event::TestConnectionOk));
        assert!(state.conn.all().is_empty(), "test conn should not be saved");
    }

    #[tokio::test]
    async fn test_connection_should_send_failed_on_invalid_url() {
        let state = Arc::new(AppState::new());
        let db = DbService::new();
        let (controller, tx_cmd, mut rx_event) = AppController::new(
            state.clone(),
            db,
            test_session(),
            test_repo().await,
            test_history().await,
            test_metadata_cache().await,
            tempdir().unwrap().keep(),
            [0u8; 32],
        );

        let bad = DbConnection {
            id: "tbad".to_string(),
            name: "tbad".to_string(),
            db_type: DbType::SQLite,
            connection_string: Some("sqlite:///no/such/path/???invalid".to_string()),
            host: None,
            port: None,
            user: None,
            password_encrypted: None,
            database: None,
            ssh: None,
        };
        tx_cmd
            .send(Command::TestConnection(bad, None))
            .await
            .unwrap();
        drop(tx_cmd);

        controller.run().await;

        let event = rx_event.recv().await.expect("expected event");
        assert!(matches!(event, Event::TestConnectionFailed(_)));
        assert!(
            state.conn.all().is_empty(),
            "failed test conn should not be saved"
        );
    }

    #[tokio::test]
    async fn connect_should_send_connected_event_on_success() {
        let state = Arc::new(AppState::new());
        let db = DbService::new();
        let (controller, tx_cmd, mut rx_event) = AppController::new(
            state.clone(),
            db,
            test_session(),
            test_repo().await,
            test_history().await,
            test_metadata_cache().await,
            tempdir().unwrap().keep(),
            [0u8; 32],
        );

        tx_cmd
            .send(Command::Connect(sqlite_conn("c1"), None))
            .await
            .unwrap();
        drop(tx_cmd);

        controller.run().await;

        let event = rx_event.recv().await.expect("expected event");
        assert!(matches!(event, Event::Connected { ref id, .. } if id == "c1"));
        assert!(state.conn.active().is_some());
    }

    #[tokio::test]
    async fn connect_should_send_connect_error_on_invalid_url() {
        let state = Arc::new(AppState::new());
        let db = DbService::new();
        let (controller, tx_cmd, mut rx_event) = AppController::new(
            state.clone(),
            db,
            test_session(),
            test_repo().await,
            test_history().await,
            test_metadata_cache().await,
            tempdir().unwrap().keep(),
            [0u8; 32],
        );

        let bad = DbConnection {
            id: "bad".to_string(),
            name: "bad".to_string(),
            db_type: DbType::SQLite,
            connection_string: Some("sqlite:///no/such/path/???invalid".to_string()),
            host: None,
            port: None,
            user: None,
            password_encrypted: None,
            database: None,
            ssh: None,
        };
        tx_cmd.send(Command::Connect(bad, None)).await.unwrap();
        drop(tx_cmd);

        controller.run().await;

        let event = rx_event.recv().await.expect("expected event");
        assert!(matches!(event, Event::ConnectError(_)));
    }

    #[tokio::test]
    async fn disconnect_should_send_disconnected_event() {
        let state = Arc::new(AppState::new());
        let db = DbService::new();
        let (controller, tx_cmd, mut rx_event) = AppController::new(
            state.clone(),
            db,
            test_session(),
            test_repo().await,
            test_history().await,
            test_metadata_cache().await,
            tempdir().unwrap().keep(),
            [0u8; 32],
        );

        tx_cmd
            .send(Command::Connect(sqlite_conn("c2"), None))
            .await
            .unwrap();
        tx_cmd
            .send(Command::Disconnect("c2".to_string()))
            .await
            .unwrap();
        drop(tx_cmd);

        tokio::spawn(controller.run());

        let e1 = rx_event.recv().await.unwrap();
        assert!(matches!(e1, Event::Connected { .. }));
        // Drain any MetadataLoaded/MetadataFetchFailed from the background fetch.
        let e2 = loop {
            match rx_event.recv().await.unwrap() {
                Event::MetadataLoaded(_, _) | Event::MetadataFetchFailed(_) => continue,
                e => break e,
            }
        };
        assert!(matches!(e2, Event::Disconnected(ref id) if id == "c2"));
    }

    // ── RunQuery ──────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn run_query_should_send_query_started_then_finished() {
        let state = Arc::new(AppState::new());
        let db = DbService::new();
        let (controller, tx_cmd, mut rx_event) = AppController::new(
            state.clone(),
            db,
            test_session(),
            test_repo().await,
            test_history().await,
            test_metadata_cache().await,
            tempdir().unwrap().keep(),
            [0u8; 32],
        );

        tx_cmd
            .send(Command::Connect(sqlite_conn("q1"), None))
            .await
            .unwrap();
        tx_cmd
            .send(Command::RunQuery("SELECT 1 AS n".to_string()))
            .await
            .unwrap();
        drop(tx_cmd);

        tokio::spawn(controller.run());

        let e1 = rx_event.recv().await.unwrap();
        assert!(matches!(e1, Event::Connected { .. }));
        let e2 = loop {
            match rx_event.recv().await.unwrap() {
                Event::MetadataLoaded(_, _) | Event::MetadataFetchFailed(_) => continue,
                e => break e,
            }
        };
        assert!(matches!(e2, Event::QueryStarted));
        let e3 = loop {
            match rx_event.recv().await.unwrap() {
                Event::MetadataLoaded(_, _) | Event::MetadataFetchFailed(_) => continue,
                e => break e,
            }
        };
        assert!(matches!(e3, Event::QueryFinished(_)));
    }

    #[tokio::test]
    async fn run_query_should_send_query_error_when_no_active_connection() {
        let state = Arc::new(AppState::new());
        let db = DbService::new();
        let (controller, tx_cmd, mut rx_event) = AppController::new(
            state.clone(),
            db,
            test_session(),
            test_repo().await,
            test_history().await,
            test_metadata_cache().await,
            tempdir().unwrap().keep(),
            [0u8; 32],
        );

        tx_cmd
            .send(Command::RunQuery("SELECT 1".to_string()))
            .await
            .unwrap();
        drop(tx_cmd);

        controller.run().await;

        let event = rx_event.recv().await.unwrap();
        assert!(matches!(event, Event::QueryError(_)));
    }

    #[tokio::test]
    async fn cancel_query_should_send_query_cancelled_event() {
        let state = Arc::new(AppState::new());
        let db = DbService::new();
        let (controller, tx_cmd, mut rx_event) = AppController::new(
            state.clone(),
            db,
            test_session(),
            test_repo().await,
            test_history().await,
            test_metadata_cache().await,
            tempdir().unwrap().keep(),
            [0u8; 32],
        );

        tx_cmd.send(Command::CancelQuery).await.unwrap();
        drop(tx_cmd);

        controller.run().await;

        let event = rx_event.recv().await.unwrap();
        assert!(matches!(event, Event::QueryCancelled));
    }

    #[tokio::test]
    async fn connect_should_send_metadata_loaded_after_connected() {
        let state = Arc::new(AppState::new());
        let db = DbService::new();
        let (controller, tx_cmd, mut rx_event) = AppController::new(
            state.clone(),
            db,
            test_session(),
            test_repo().await,
            test_history().await,
            test_metadata_cache().await,
            tempdir().unwrap().keep(),
            [0u8; 32],
        );

        tx_cmd
            .send(Command::Connect(sqlite_conn("meta-1"), None))
            .await
            .unwrap();
        drop(tx_cmd);

        tokio::spawn(controller.run());

        let e1 = rx_event.recv().await.unwrap();
        assert!(matches!(e1, Event::Connected { .. }));
        let e2 = rx_event.recv().await.unwrap();
        assert!(
            matches!(
                e2,
                Event::MetadataLoaded(_, _) | Event::MetadataFetchFailed(_)
            ),
            "expected MetadataLoaded or MetadataFetchFailed, got {e2:?}"
        );
    }

    #[tokio::test]
    async fn connect_twice_should_not_duplicate_in_state() {
        let state = Arc::new(AppState::new());
        let db = DbService::new();
        let (controller, tx_cmd, mut rx_event) = AppController::new(
            state.clone(),
            db,
            test_session(),
            test_repo().await,
            test_history().await,
            test_metadata_cache().await,
            tempdir().unwrap().keep(),
            [0u8; 32],
        );

        tx_cmd
            .send(Command::Connect(sqlite_conn("c3"), None))
            .await
            .unwrap();
        tx_cmd
            .send(Command::Connect(sqlite_conn("c3"), None))
            .await
            .unwrap();
        drop(tx_cmd);

        tokio::spawn(controller.run());

        let mut connected_count = 0;
        while connected_count < 2 {
            match rx_event.recv().await.unwrap() {
                Event::Connected { .. } => connected_count += 1,
                Event::MetadataLoaded(_, _) | Event::MetadataFetchFailed(_) => {}
                _ => {}
            }
        }

        assert_eq!(state.conn.all().len(), 1, "conn should not be duplicated");
    }
}

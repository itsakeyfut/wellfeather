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

use std::path::PathBuf;

use chrono::Utc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};
use wf_completion::{cache::MetadataCache, service::CompletionService};
use wf_db::{error::DbError, models::DbConnection, service::DbService};
use wf_history::service::HistoryService;

use crate::{
    app::{
        LocalizedMessage,
        command::{Command, ConfigUpdate},
        event::{Event, StateEvent},
        session::SessionManager,
    },
    state::SharedState,
};

/// Async command loop that drives the application backend.
///
const CMD_CHANNEL_CAPACITY: usize = 64;

/// Owns the [`DbService`] pool, the [`SessionManager`] for config persistence,
/// and the [`SharedState`] shared with the UI layer. Created by [`AppController::new`]
/// and consumed by [`AppController::run`], which is spawned as a tokio task.
pub struct AppController {
    state: SharedState,
    db: DbService,
    session: SessionManager,
    /// Path to `history.db`; opened asynchronously at the start of `run()`.
    history_path: PathBuf,
    /// `None` until `run()` opens the database; failures are non-fatal (logged).
    history: Option<HistoryService>,
    /// Path to `metadata.db`; opened asynchronously at the start of `run()`.
    metadata_cache_path: PathBuf,
    /// `None` until `run()` initialises the cache.
    metadata_cache: Option<MetadataCache>,
    /// `None` until `run()` initialises the cache (same `MetadataCache` clone).
    completion: Option<CompletionService>,
    rx_cmd: mpsc::Receiver<Command>,
    tx_event: mpsc::Sender<Event>,
}

impl AppController {
    /// Create the controller and return it together with the two channel endpoints
    /// that `main.rs` distributes: `Sender<Command>` → UI, `Receiver<Event>` → UI.
    ///
    /// `history_path` is the filesystem path for `history.db`; the database is
    /// opened (and the schema migrated) asynchronously at the start of [`Self::run`].
    /// `metadata_cache_path` is the filesystem path for `metadata.db`.
    pub fn new(
        state: SharedState,
        db: DbService,
        session: SessionManager,
        history_path: PathBuf,
        metadata_cache_path: PathBuf,
    ) -> (Self, mpsc::Sender<Command>, mpsc::Receiver<Event>) {
        let (tx_cmd, rx_cmd) = mpsc::channel(CMD_CHANNEL_CAPACITY);
        let (tx_event, rx_event) = mpsc::channel(CMD_CHANNEL_CAPACITY);
        (
            Self {
                state,
                db,
                session,
                history_path,
                history: None,
                metadata_cache_path,
                metadata_cache: None,
                completion: None,
                rx_cmd,
                tx_event,
            },
            tx_cmd,
            rx_event,
        )
    }

    /// Run the command loop as a tokio task (spawn with `tokio::spawn(controller.run())`).
    /// Exits when all `Sender<Command>` clones are dropped.
    pub async fn run(mut self) {
        // Open history.db at startup — failure is non-fatal (queries still work).
        self.history = match HistoryService::open(&self.history_path).await {
            Ok(h) => {
                info!("history.db opened at {:?}", self.history_path);
                Some(h)
            }
            Err(e) => {
                warn!("failed to open history.db: {e}");
                None
            }
        };

        // Open metadata cache at startup — failure is non-fatal.
        let cache = MetadataCache::new(self.metadata_cache_path.clone());
        if let Err(e) = cache.preload_from_disk().await {
            warn!("failed to preload metadata cache: {e}");
        }
        self.completion = Some(CompletionService::new(cache.clone()));
        self.metadata_cache = Some(cache);

        while let Some(cmd) = self.rx_cmd.recv().await {
            debug!("received command: {:?}", cmd);
            match cmd {
                Command::Connect(conn, pw) => self.handle_connect(conn, pw).await,
                Command::TestConnection(conn, pw) => self.handle_test_connection(conn, pw).await,
                Command::Disconnect(id) => self.handle_disconnect(id).await,
                Command::RunQuery(sql) => self.handle_run_query(sql).await,
                Command::RunAll(sql) => self.handle_run_query(sql).await,
                Command::RunSelection(sql) => self.handle_run_query(sql).await,
                Command::CancelQuery => self.handle_cancel_query().await,
                Command::UpdateConfig(update) => self.handle_update_config(update).await,
                Command::FetchCompletion(sql, cursor_pos) => {
                    self.handle_fetch_completion(sql, cursor_pos).await;
                }
                _ => {} // remaining commands handled in later tasks
            }
        }
    }

    /// Handle a `Connect` command.
    ///
    /// On success:
    /// 1. Persists the session via [`SessionManager::save_connection`].
    /// 2. Adds the connection to [`SharedState`] if it is not already present.
    /// 3. Marks it as the active connection.
    /// 4. Sends [`Event::Connected`] to the UI.
    ///
    /// On failure, sends [`Event::QueryError`] with the error message.
    async fn handle_connect(&self, conn: DbConnection, password: Option<String>) {
        let id = conn.id.clone();
        info!(conn_id = %id, "handling Connect command");
        match self.db.connect(&conn, password.as_deref()).await {
            Ok(()) => {
                // persist session so we can auto-reconnect on next launch
                if let Err(e) = self.session.save_connection(&conn) {
                    warn!(conn_id = %id, error = %e, "failed to save session");
                }
                // Only add to the saved list if this is a new connection.
                let already_saved = self.state.conn.all().iter().any(|c| c.id == id);
                if !already_saved {
                    self.state.conn.add(conn);
                }
                self.state.conn.set_active(&id);
                info!(conn_id = %id, "connected successfully");
                let _ = self.tx_event.send(Event::Connected(id.clone())).await;

                let db = self.db.clone(); // clone required: tokio::spawn needs 'static
                let tx = self.tx_event.clone(); // clone required: tokio::spawn needs 'static
                let cache = self.metadata_cache.clone(); // clone required: tokio::spawn needs 'static
                let fetch_id = id.clone(); // clone required: owned id for async block
                tokio::spawn(async move {
                    match db.fetch_metadata(&fetch_id).await {
                        Ok(meta) => {
                            if let Some(ref c) = cache
                                && let Err(e) = c.store(&fetch_id, meta.clone()).await
                            {
                                warn!(conn_id = %fetch_id, error = %e, "failed to store metadata");
                            }
                            let _ = tx.send(Event::MetadataLoaded(fetch_id.clone(), meta)).await;
                        }
                        Err(e) => {
                            warn!(conn_id = %fetch_id, error = %e, "metadata fetch failed");
                            let _ = tx.send(Event::MetadataFetchFailed(e.to_string())).await;
                        }
                    }
                });
            }
            Err(e) => {
                warn!(conn_id = %id, error = %e, "connection failed");
                let _ = self
                    .tx_event
                    .send(Event::ConnectError(e.localized_message()))
                    .await;
            }
        }
    }

    /// Handle a `TestConnection` command.
    ///
    /// Tries to establish a connection, then immediately drops it.
    /// Does **not** add the connection to [`SharedState`] or the sidebar.
    /// Sends [`Event::TestConnectionOk`] on success or
    /// [`Event::TestConnectionFailed`] on failure.
    async fn handle_test_connection(&self, conn: DbConnection, password: Option<String>) {
        let id = conn.id.clone();
        info!(conn_id = %id, "handling TestConnection command");
        match self.db.connect(&conn, password.as_deref()).await {
            Ok(()) => {
                // Drop the temporary pool immediately — do not persist to state.
                self.db.disconnect(&id);
                info!(conn_id = %id, "test connection succeeded");
                let _ = self.tx_event.send(Event::TestConnectionOk).await;
            }
            Err(e) => {
                warn!(conn_id = %id, error = %e, "test connection failed");
                let _ = self
                    .tx_event
                    .send(Event::TestConnectionFailed(e.localized_message()))
                    .await;
            }
        }
    }

    /// Handle a `Disconnect` command.
    ///
    /// Drops the connection pool for `id` and sends [`Event::Disconnected`] to the UI.
    async fn handle_disconnect(&self, id: String) {
        info!(conn_id = %id, "handling Disconnect command");
        self.db.disconnect(&id);
        let _ = self.tx_event.send(Event::Disconnected(id)).await;
    }

    /// Handle a `RunQuery` / `RunAll` / `RunSelection` command.
    ///
    /// Steps:
    /// 1. Cancel any in-flight query via `QueryState::cancel`.
    /// 2. Bail with [`Event::QueryError`] if there is no active connection.
    /// 3. Create a fresh [`CancellationToken`] and register it in `QueryState`.
    /// 4. Send [`Event::QueryStarted`] to the UI immediately.
    /// 5. Spawn a background task that calls [`DbService::execute_with_cancel`]
    ///    and sends [`Event::QueryFinished`] / [`Event::QueryCancelled`] /
    ///    [`Event::QueryError`] when done.
    async fn handle_run_query(&self, sql: String) {
        info!("handling RunQuery command");
        self.state.query.cancel();

        let conn_id = match self.state.conn.active() {
            Some(c) => c.id.clone(),
            None => {
                warn!("RunQuery: no active connection");
                let _ = self
                    .tx_event
                    .send(Event::QueryError(crate::app::locale::tr(
                        "error.no_active_connection",
                        &[],
                    )))
                    .await;
                return;
            }
        };

        self.state.query.set_last_sql(sql.clone());

        let token = CancellationToken::new();
        self.state.query.set_cancel_token(token.clone());
        debug!("sending event: QueryStarted");
        let _ = self.tx_event.send(Event::QueryStarted).await;

        let page_size = self.state.ui.page_size();
        let sql_to_run = apply_limit(&sql, page_size);

        let db = self.db.clone(); // clone required: tokio::spawn needs 'static
        let tx = self.tx_event.clone(); // clone required: tokio::spawn needs 'static
        let history = self.history.clone(); // clone required: tokio::spawn needs 'static
        let sql_hist = sql.clone(); // clone required: history record needs owned sql
        let conn_id_hist = conn_id.clone(); // clone required: history record needs owned id
        tokio::spawn(async move {
            let now = Utc::now().timestamp();
            match db.execute_with_cancel(&conn_id, &sql_to_run, token).await {
                Ok(result) => {
                    if let Some(ref h) = history {
                        let exec = wf_db::models::QueryExecution {
                            id: 0,
                            sql: sql_hist,
                            duration_ms: result.execution_time_ms,
                            success: true,
                            error_message: None,
                            timestamp: now,
                            connection_id: conn_id_hist,
                        };
                        if let Err(e) = h.insert(&exec).await {
                            warn!("failed to save history: {e}");
                        }
                    }
                    debug!("sending event: QueryFinished");
                    let _ = tx.send(Event::QueryFinished(result)).await;
                }
                Err(DbError::Cancelled) => {
                    debug!("sending event: QueryCancelled");
                    let _ = tx.send(Event::QueryCancelled).await;
                }
                Err(e) => {
                    error!(error = %e, "query execution failed");
                    if let Some(ref h) = history {
                        let exec = wf_db::models::QueryExecution {
                            id: 0,
                            sql: sql_hist,
                            duration_ms: 0,
                            success: false,
                            error_message: Some(e.to_string()),
                            timestamp: now,
                            connection_id: conn_id_hist,
                        };
                        if let Err(he) = h.insert(&exec).await {
                            warn!("failed to save history: {he}");
                        }
                    }
                    debug!("sending event: QueryError");
                    let _ = tx.send(Event::QueryError(e.localized_message())).await;
                }
            }
        });
    }

    /// Handle an `UpdateConfig` command.
    ///
    /// Handles `Theme` and `PageSize` changes: updates shared state so they
    /// survive the current session, then persists the value to `config.toml`.
    async fn handle_update_config(&self, update: ConfigUpdate) {
        match update {
            ConfigUpdate::Theme(t) => {
                self.state.ui.set_theme(t.clone());
                if let Err(e) = self.session.save_theme(&t) {
                    warn!(error = %e, "failed to persist theme to config");
                }
                let _ = self
                    .tx_event
                    .send(Event::StateChanged(StateEvent::ThemeChanged(t)))
                    .await;
            }
            ConfigUpdate::PageSize(ps) => {
                let n: u32 = ps.into();
                self.state.ui.set_page_size(n as usize);
                if let Err(e) = self.session.save_page_size(n as usize) {
                    warn!(error = %e, "failed to persist page_size to config");
                }
                let _ = self.tx_event.send(Event::ConfigUpdated).await;
            }
            ConfigUpdate::Language(lang) => {
                if let Err(e) = self.session.save_language(&lang) {
                    warn!(error = %e, "failed to persist language to config");
                }
            }
            _ => {}
        }
    }

    /// Handle a `FetchCompletion` command.
    ///
    /// Looks up the active connection, calls [`CompletionService::complete`], and
    /// sends [`Event::CompletionReady`] with the (possibly empty) candidate list.
    /// Silently no-ops when there is no active connection or no completion service.
    async fn handle_fetch_completion(&self, sql: String, cursor_pos: usize) {
        let conn_id = match self.state.conn.active() {
            Some(c) => c.id.clone(),
            None => return,
        };
        if let Some(ref completion) = self.completion {
            let items = completion.complete(&conn_id, &sql, cursor_pos).await;
            let _ = self.tx_event.send(Event::CompletionReady(items)).await;
        }
    }

    /// Handle a `CancelQuery` command.
    ///
    /// Fires the stored [`CancellationToken`] (if any) and immediately sends
    /// [`Event::QueryCancelled`] to the UI so it can reset its loading state.
    async fn handle_cancel_query(&self) {
        info!("handling CancelQuery command");
        self.state.query.cancel();
        let _ = self.tx_event.send(Event::QueryCancelled).await;
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
    // Multi-statement SQL (semicolon in the middle) must not have LIMIT injected;
    // the caller is responsible for splitting statements before calling apply_limit.
    if trimmed.contains(';') {
        return sql.to_string();
    }
    let upper = trimmed.to_uppercase();
    if upper.starts_with("SELECT") && !upper.contains(" LIMIT ") {
        format!("{} LIMIT {}", trimmed, limit)
    } else {
        sql.to_string()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, sync::Arc};

    use tempfile::tempdir;
    use wf_config::manager::ConfigManager;
    use wf_db::{
        models::{DbConnection, DbType},
        service::DbService,
    };

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

    /// Return a path to a temporary `history.db` (file created lazily by the controller).
    fn test_history_path() -> PathBuf {
        let dir = tempdir().unwrap();
        dir.keep().join("history.db")
    }

    /// Return a path to a temporary `metadata.db` (file created lazily by the controller).
    fn test_metadata_path() -> PathBuf {
        let dir = tempdir().unwrap();
        dir.keep().join("metadata.db")
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
        }
    }

    // ── apply_limit ───────────────────────────────────────────────────────────

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
        let multi = "SELECT * FROM a;\nSELECT * FROM b";
        assert_eq!(apply_limit(multi, 500), multi);
    }

    // ── TestConnection ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_connection_should_send_ok_and_not_add_to_state() {
        let state = Arc::new(AppState::new());
        let db = DbService::new();
        let (controller, tx_cmd, mut rx_event) = AppController::new(
            state.clone(),
            db,
            test_session(),
            test_history_path(),
            test_metadata_path(),
        );

        tx_cmd
            .send(Command::TestConnection(sqlite_conn("t1"), None))
            .await
            .unwrap();
        drop(tx_cmd);

        controller.run().await;

        let event = rx_event.recv().await.expect("expected event");
        assert!(matches!(event, Event::TestConnectionOk));
        // Must NOT be added to state
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
            test_history_path(),
            test_metadata_path(),
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
            test_history_path(),
            test_metadata_path(),
        );

        tx_cmd
            .send(Command::Connect(sqlite_conn("c1"), None))
            .await
            .unwrap();
        drop(tx_cmd); // close channel → run() exits after processing

        controller.run().await;

        let event = rx_event.recv().await.expect("expected event");
        assert!(matches!(event, Event::Connected(ref id) if id == "c1"));
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
            test_history_path(),
            test_metadata_path(),
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
            test_history_path(),
            test_metadata_path(),
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
        assert!(matches!(e1, Event::Connected(_)));
        // Drain any MetadataLoaded/MetadataFetchFailed from the background fetch.
        let e2 = loop {
            match rx_event.recv().await.unwrap() {
                Event::MetadataLoaded(_, _) | Event::MetadataFetchFailed(_) => continue,
                e => break e,
            }
        };
        assert!(matches!(e2, Event::Disconnected(ref id) if id == "c2"));
    }

    // ── RunQuery ──────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn run_query_should_send_query_started_then_finished() {
        let state = Arc::new(AppState::new());
        let db = DbService::new();
        let (controller, tx_cmd, mut rx_event) = AppController::new(
            state.clone(),
            db,
            test_session(),
            test_history_path(),
            test_metadata_path(),
        );

        // Connect first so there is an active connection.
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
        assert!(matches!(e1, Event::Connected(_)));
        // Drain any MetadataLoaded/MetadataFetchFailed before QueryStarted.
        let e2 = loop {
            match rx_event.recv().await.unwrap() {
                Event::MetadataLoaded(_, _) | Event::MetadataFetchFailed(_) => continue,
                e => break e,
            }
        };
        assert!(matches!(e2, Event::QueryStarted));
        // Drain any late MetadataLoaded/MetadataFetchFailed before QueryFinished.
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
            test_history_path(),
            test_metadata_path(),
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
            test_history_path(),
            test_metadata_path(),
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
            test_history_path(),
            test_metadata_path(),
        );

        tx_cmd
            .send(Command::Connect(sqlite_conn("meta-1"), None))
            .await
            .unwrap();
        drop(tx_cmd);

        tokio::spawn(controller.run());

        let e1 = rx_event.recv().await.unwrap();
        assert!(matches!(e1, Event::Connected(_)));
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
            test_history_path(),
            test_metadata_path(),
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

        // Drain the 2 Connected events plus any MetadataLoaded/MetadataFetchFailed events.
        let mut connected_count = 0;
        while connected_count < 2 {
            match rx_event.recv().await.unwrap() {
                Event::Connected(_) => connected_count += 1,
                Event::MetadataLoaded(_, _) | Event::MetadataFetchFailed(_) => {}
                _ => {}
            }
        }

        assert_eq!(state.conn.all().len(), 1, "conn should not be duplicated");
    }
}

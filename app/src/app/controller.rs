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
//! Both channels are bounded (`capacity = 64`). The controller task exits cleanly
//! when all `Sender<Command>` clones are dropped (i.e. when the UI window closes).

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use wf_db::{error::DbError, models::DbConnection, service::DbService};

use crate::{
    app::{command::Command, event::Event, session::SessionManager},
    state::SharedState,
};

/// Async command loop that drives the application backend.
///
/// Owns the [`DbService`] pool, the [`SessionManager`] for config persistence,
/// and the [`SharedState`] shared with the UI layer. Created by [`AppController::new`]
/// and consumed by [`AppController::run`], which is spawned as a tokio task.
pub struct AppController {
    state: SharedState,
    db: DbService,
    session: SessionManager,
    rx_cmd: mpsc::Receiver<Command>,
    tx_event: mpsc::Sender<Event>,
}

impl AppController {
    /// Create the controller and return it together with the two channel endpoints
    /// that `main.rs` distributes: `Sender<Command>` → UI, `Receiver<Event>` → UI.
    pub fn new(
        state: SharedState,
        db: DbService,
        session: SessionManager,
    ) -> (Self, mpsc::Sender<Command>, mpsc::Receiver<Event>) {
        let (tx_cmd, rx_cmd) = mpsc::channel(64);
        let (tx_event, rx_event) = mpsc::channel(64);
        (
            Self {
                state,
                db,
                session,
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
        while let Some(cmd) = self.rx_cmd.recv().await {
            match cmd {
                Command::Connect(conn, pw) => self.handle_connect(conn, pw).await,
                Command::TestConnection(conn, pw) => self.handle_test_connection(conn, pw).await,
                Command::Disconnect(id) => self.handle_disconnect(id).await,
                Command::RunQuery(sql) => self.handle_run_query(sql).await,
                Command::RunAll(sql) => self.handle_run_query(sql).await,
                Command::RunSelection(sql) => self.handle_run_query(sql).await,
                Command::CancelQuery => self.handle_cancel_query().await,
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
                let _ = self.tx_event.send(Event::Connected(id)).await;
            }
            Err(e) => {
                warn!(conn_id = %id, error = %e, "connection failed");
                let _ = self.tx_event.send(Event::ConnectError(e.to_string())).await;
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
                    .send(Event::TestConnectionFailed(e.to_string()))
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
                    .send(Event::QueryError("no active connection".to_string()))
                    .await;
                return;
            }
        };

        let token = CancellationToken::new();
        self.state.query.set_cancel_token(token.clone());
        let _ = self.tx_event.send(Event::QueryStarted).await;

        let db = self.db.clone(); // clone required: tokio::spawn needs 'static
        let tx = self.tx_event.clone(); // clone required: tokio::spawn needs 'static
        tokio::spawn(async move {
            match db.execute_with_cancel(&conn_id, &sql, token).await {
                Ok(result) => {
                    let _ = tx.send(Event::QueryFinished(result)).await;
                }
                Err(DbError::Cancelled) => {
                    let _ = tx.send(Event::QueryCancelled).await;
                }
                Err(e) => {
                    let _ = tx.send(Event::QueryError(e.to_string())).await;
                }
            }
        });
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;

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

    use super::AppController;

    /// Build a [`SessionManager`] backed by a temporary directory.
    /// `keep()` prevents cleanup so the path stays valid for the test lifetime.
    fn test_session() -> SessionManager {
        let dir = tempdir().unwrap();
        let path = dir.keep().join("config.toml");
        SessionManager::with_config_manager(ConfigManager::with_path(path))
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

    // ── TestConnection ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_connection_should_send_ok_and_not_add_to_state() {
        let state = Arc::new(AppState::new());
        let db = DbService::new();
        let (controller, tx_cmd, mut rx_event) =
            AppController::new(state.clone(), db, test_session());

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
        let (controller, tx_cmd, mut rx_event) =
            AppController::new(state.clone(), db, test_session());

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
        let (controller, tx_cmd, mut rx_event) =
            AppController::new(state.clone(), db, test_session());

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
        let (controller, tx_cmd, mut rx_event) =
            AppController::new(state.clone(), db, test_session());

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
        let (controller, tx_cmd, mut rx_event) =
            AppController::new(state.clone(), db, test_session());

        tx_cmd
            .send(Command::Connect(sqlite_conn("c2"), None))
            .await
            .unwrap();
        tx_cmd
            .send(Command::Disconnect("c2".to_string()))
            .await
            .unwrap();
        drop(tx_cmd);

        controller.run().await;

        let e1 = rx_event.recv().await.unwrap();
        assert!(matches!(e1, Event::Connected(_)));
        let e2 = rx_event.recv().await.unwrap();
        assert!(matches!(e2, Event::Disconnected(ref id) if id == "c2"));
    }

    // ── RunQuery ──────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn run_query_should_send_query_started_then_finished() {
        let state = Arc::new(AppState::new());
        let db = DbService::new();
        let (controller, tx_cmd, mut rx_event) =
            AppController::new(state.clone(), db, test_session());

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
        let e2 = rx_event.recv().await.unwrap();
        assert!(matches!(e2, Event::QueryStarted));
        let e3 = rx_event.recv().await.unwrap();
        assert!(matches!(e3, Event::QueryFinished(_)));
    }

    #[tokio::test]
    async fn run_query_should_send_query_error_when_no_active_connection() {
        let state = Arc::new(AppState::new());
        let db = DbService::new();
        let (controller, tx_cmd, mut rx_event) =
            AppController::new(state.clone(), db, test_session());

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
        let (controller, tx_cmd, mut rx_event) =
            AppController::new(state.clone(), db, test_session());

        tx_cmd.send(Command::CancelQuery).await.unwrap();
        drop(tx_cmd);

        controller.run().await;

        let event = rx_event.recv().await.unwrap();
        assert!(matches!(event, Event::QueryCancelled));
    }

    #[tokio::test]
    async fn connect_twice_should_not_duplicate_in_state() {
        let state = Arc::new(AppState::new());
        let db = DbService::new();
        let (controller, tx_cmd, mut rx_event) =
            AppController::new(state.clone(), db, test_session());

        tx_cmd
            .send(Command::Connect(sqlite_conn("c3"), None))
            .await
            .unwrap();
        tx_cmd
            .send(Command::Connect(sqlite_conn("c3"), None))
            .await
            .unwrap();
        drop(tx_cmd);

        controller.run().await;

        // drain events
        rx_event.recv().await.unwrap();
        rx_event.recv().await.unwrap();

        assert_eq!(state.conn.all().len(), 1, "conn should not be duplicated");
    }
}

use tokio::sync::mpsc;
use tracing::{info, warn};
use wf_db::{models::DbConnection, service::DbService};

use crate::{
    app::{command::Command, event::Event},
    state::SharedState,
};

pub struct AppController {
    state: SharedState,
    db: DbService,
    rx_cmd: mpsc::Receiver<Command>,
    tx_event: mpsc::Sender<Event>,
}

impl AppController {
    /// Create the controller and return it together with the two channel endpoints
    /// that `main.rs` distributes: `Sender<Command>` → UI, `Receiver<Event>` → UI.
    pub fn new(
        state: SharedState,
        db: DbService,
    ) -> (Self, mpsc::Sender<Command>, mpsc::Receiver<Event>) {
        let (tx_cmd, rx_cmd) = mpsc::channel(64);
        let (tx_event, rx_event) = mpsc::channel(64);
        (
            Self {
                state,
                db,
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
                Command::Disconnect(id) => self.handle_disconnect(id).await,
                _ => {} // remaining commands handled in later tasks
            }
        }
    }

    async fn handle_connect(&self, conn: DbConnection, password: Option<String>) {
        let id = conn.id.clone();
        info!(conn_id = %id, "handling Connect command");
        match self.db.connect(&conn, password.as_deref()).await {
            Ok(()) => {
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
                let _ = self.tx_event.send(Event::QueryError(e.to_string())).await;
            }
        }
    }

    async fn handle_disconnect(&self, id: String) {
        info!(conn_id = %id, "handling Disconnect command");
        self.db.disconnect(&id);
        let _ = self.tx_event.send(Event::Disconnected(id)).await;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use wf_db::{
        models::{DbConnection, DbType},
        service::DbService,
    };

    use crate::{
        app::{command::Command, event::Event},
        state::AppState,
    };

    use super::AppController;

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

    #[tokio::test]
    async fn connect_should_send_connected_event_on_success() {
        let state = Arc::new(AppState::new());
        let db = DbService::new();
        let (controller, tx_cmd, mut rx_event) = AppController::new(state.clone(), db);

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
    async fn connect_should_send_query_error_on_invalid_url() {
        let state = Arc::new(AppState::new());
        let db = DbService::new();
        let (controller, tx_cmd, mut rx_event) = AppController::new(state.clone(), db);

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
        assert!(matches!(event, Event::QueryError(_)));
    }

    #[tokio::test]
    async fn disconnect_should_send_disconnected_event() {
        let state = Arc::new(AppState::new());
        let db = DbService::new();
        let (controller, tx_cmd, mut rx_event) = AppController::new(state.clone(), db);

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

    #[tokio::test]
    async fn connect_twice_should_not_duplicate_in_state() {
        let state = Arc::new(AppState::new());
        let db = DbService::new();
        let (controller, tx_cmd, mut rx_event) = AppController::new(state.clone(), db);

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

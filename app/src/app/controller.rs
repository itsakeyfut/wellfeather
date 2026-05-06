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
#[cfg(test)]
mod tests;

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
}

impl AppController {
    /// Create the controller and return it together with the two channel endpoints
    /// that `main.rs` distributes: `Sender<Command>` → UI, `Receiver<Event>` → UI.
    ///
    /// All services are expected to be fully initialised (schema migrations run)
    /// before being passed in. The shared `SqlitePool` backing them is managed by
    /// the caller (`main.rs`).
    pub fn new(
        state: SharedState,
        db: DbService,
        session: SessionManager,
        repo: Arc<ConnectionRepository>,
        history: HistoryService,
        metadata_cache: MetadataCache,
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
            debug!("received command: {:?}", cmd);
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
    if upper.starts_with("SELECT") && !upper.contains(" LIMIT ") && !trimmed.contains(';') {
        format!("{} LIMIT {}", trimmed, limit)
    } else {
        sql.to_string()
    }
}

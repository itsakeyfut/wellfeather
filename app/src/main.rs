//! Application entry point.
//!
//! Responsibilities:
//!
//! 1. Initialise tracing and the tokio multi-thread runtime.
//! 2. Construct the shared [`AppState`], [`DbService`], and [`SessionManager`].
//! 3. Attempt to restore the previous session and schedule an auto-connect if one exists.
//! 4. Spawn the [`AppController`] command loop.
//! 5. Build and run the Slint UI on the main thread (required by most windowing systems).
//!
//! # Channel topology
//!
//! ```text
//! main ──(tx_cmd)──▶ AppController ──(tx_event)──▶ UI::spawn_event_handler
//!        ◀──────────────────────────────────────────(rx_event)──
//! ```

slint::include_modules!();

mod app;
mod state;
mod ui;

use std::sync::Arc;

use app::{controller::AppController, session::SessionManager};
use state::AppState;
use ui::UI;
use wf_db::service::DbService;

/// Entry point. Runs on the main OS thread; the Slint event loop must stay here.
fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    // Keep the runtime context active on the main thread so tokio::spawn
    // calls from Slint callbacks and the event-handler task work without
    // an explicit runtime handle.
    let _guard = runtime.enter();

    let state = Arc::new(AppState::new());
    let db = DbService::new();

    let session = SessionManager::new();

    // Attempt to restore the last session before the event loop starts.
    let restore_conn = match session.restore() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("session restore failed: {e}");
            None
        }
    };

    let (controller, tx_cmd, rx_event) = AppController::new(state.clone(), db, session);
    tokio::spawn(controller.run());

    // Send auto-connect before entering the event loop.
    if let Some(conn) = restore_conn {
        // clone required: tx_cmd also passed to UI
        let tx = tx_cmd.clone();
        tokio::spawn(async move {
            let _ = tx.send(app::command::Command::Connect(conn, None)).await;
        });
    }

    let ui = UI::new(state, tx_cmd, rx_event)?;
    ui.run()
}

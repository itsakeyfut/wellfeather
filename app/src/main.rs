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
use wf_config::{crypto, manager::ConfigManager};
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

    // Load (or generate) the AES-256-GCM key used to encrypt stored passwords.
    let enc_key = crypto::load_or_create_key(&ConfigManager::app_dir())?;

    let state = Arc::new(AppState::new());

    // Load persisted page_size and theme from config so the first launch uses the user's last settings.
    {
        let config = ConfigManager::new().load().unwrap_or_default();
        state
            .ui
            .set_page_size(u32::from(config.editor.page_size) as usize);
        state.ui.set_theme(config.appearance.theme);
    }

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

    let (controller, tx_cmd, rx_event) = AppController::new(
        state.clone(),
        db,
        session,
        ConfigManager::app_dir().join("history.db"),
        ConfigManager::app_dir().join("metadata.db"),
    );
    tokio::spawn(controller.run());

    // Send auto-connect before entering the event loop.
    // Decrypt the stored password so the controller can build the connection URL.
    if let Some(conn) = restore_conn {
        let password = conn
            .password_encrypted
            .as_ref()
            .and_then(|enc| crypto::decrypt(enc, &enc_key).ok());
        // clone required: tx_cmd also passed to UI
        let tx = tx_cmd.clone();
        tokio::spawn(async move {
            let _ = tx
                .send(app::command::Command::Connect(conn, password))
                .await;
        });
    }

    let ui = UI::new(state, tx_cmd, rx_event, enc_key)?;
    ui.run()
}

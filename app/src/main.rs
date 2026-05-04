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
rust_i18n::i18n!("locales", fallback = "en");

mod app;
mod state;
mod ui;

use std::sync::Arc;

use app::{
    controller::AppController,
    session::{SessionManager, config_to_db_conn},
};
use state::AppState;
use ui::UI;
use wf_completion::cache::MetadataCache;
use wf_config::{ConnectionRepository, SnippetRepository, crypto, manager::ConfigManager};
use wf_db::service::DbService;
use wf_history::{
    find_history::FindHistoryService, service::HistoryService, session::SessionService,
};

/// Entry point. Runs on the main OS thread; the Slint event loop must stay here.
fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

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
    // Language locale is applied in UI::new() after the Slint component is created — Slint requires
    // a live component to exist before select_bundled_translation() takes effect.
    {
        let config = ConfigManager::new().load().unwrap_or_default();
        state
            .ui
            .set_page_size(u32::from(config.editor.page_size) as usize);
        state.ui.set_theme(config.appearance.theme);
    }

    // Open the single shared SQLite database for all persistence needs.
    let pool = runtime.block_on(async {
        sqlx::sqlite::SqlitePoolOptions::new()
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(ConfigManager::app_dir().join("wellfeather.db"))
                    .create_if_missing(true)
                    .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal),
            )
            .await
    })?;

    // Initialise all services from the shared pool (Composition Root).
    let repo: Arc<ConnectionRepository> =
        Arc::new(runtime.block_on(ConnectionRepository::new(pool.clone()))?);
    let history_svc = runtime.block_on(HistoryService::new(pool.clone()))?;
    let find_history_svc = runtime.block_on(FindHistoryService::new(pool.clone()))?;
    let session_svc = runtime.block_on(SessionService::new(pool.clone()))?;
    let snippet_repo: Arc<SnippetRepository> =
        Arc::new(runtime.block_on(SnippetRepository::new(pool.clone()))?);
    let metadata_cache = runtime.block_on(MetadataCache::new(pool.clone()))?;

    // Load all saved connections for the initial sidebar/DB-manager list.
    let initial_connections = runtime.block_on(repo.all()).unwrap_or_default();

    // Find the most-recently-used connection for auto-connect.
    let restore_conn = runtime
        .block_on(repo.last_used())
        .ok()
        .flatten()
        .map(|cc| config_to_db_conn(&cc));

    let db = DbService::new();
    let session = SessionManager::new();

    let (controller, tx_cmd, rx_event) = AppController::new(
        state.clone(),
        db,
        session,
        repo,
        history_svc,
        metadata_cache,
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

    let ui = UI::new(
        state,
        tx_cmd,
        rx_event,
        enc_key,
        initial_connections,
        find_history_svc,
        session_svc,
        snippet_repo,
    )?;
    ui.run()
}

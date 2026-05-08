#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
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
mod platform;
mod state;
mod ui;

use std::sync::Arc;

use anyhow::Context as _;
use app::{
    controller::AppController,
    session::{SessionManager, config_to_db_conn},
};
use platform::{CurrentPlatform, Platform};
use state::AppState;
use ui::UI;
use wf_completion::cache::MetadataCache;
use wf_config::{
    ConnectionRepository, SnippetRepository, crypto, manager::ConfigManager, models::Theme,
};
use wf_db::service::DbService;
use wf_history::{
    find_history::FindHistoryService, service::HistoryService, session::SessionService,
};

/// Entry point. Runs on the main OS thread; the Slint event loop must stay here.
fn main() -> anyhow::Result<()> {
    // Initialise tracing first so that enable_dpi_awareness() can log warnings.
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    // Enable per-monitor DPI awareness before any window is created (Windows only;
    // no-op on macOS and Linux where scaling is handled by the OS / desktop environment).
    platform::enable_dpi_awareness();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    // Keep the runtime context active on the main thread so tokio::spawn
    // calls from Slint callbacks and the event-handler task work without
    // an explicit runtime handle.
    let _guard = runtime.enter();

    // Resolve OS-specific directories once at startup and ensure they exist.
    // The SQLite database is intentionally kept in config_dir (same as ConfigManager::app_dir())
    // for backward compatibility with existing installations.
    let config_dir = CurrentPlatform::get_config_dir()
        .context("cannot resolve OS application config directory")?;
    let data_dir =
        CurrentPlatform::get_data_dir().context("cannot resolve OS application data directory")?;
    let cache_dir = CurrentPlatform::get_cache_dir()
        .context("cannot resolve OS application cache directory")?;
    std::fs::create_dir_all(&config_dir).context("cannot create application config directory")?;
    std::fs::create_dir_all(&data_dir).context("cannot create application data directory")?;
    std::fs::create_dir_all(&cache_dir).context("cannot create application cache directory")?;
    tracing::debug!(
        config = %config_dir.display(),
        data   = %data_dir.display(),
        cache  = %cache_dir.display(),
        "application directories resolved"
    );

    // Detect whether this is a first launch (no saved config yet) for dark-mode auto-detection.
    let config_file_exists = config_dir.join("config.toml").exists();

    // Load (or generate) the AES-256-GCM key used to encrypt stored passwords.
    let enc_key = crypto::load_or_create_key(&config_dir)?;

    let state = Arc::new(AppState::new());

    // Load persisted page_size and theme from config so the first launch uses the user's last settings.
    // Language locale is applied in UI::new() after the Slint component is created — Slint requires
    // a live component to exist before select_bundled_translation() takes effect.
    {
        let config = ConfigManager::new().load().unwrap_or_default();
        state
            .ui
            .set_page_size(u32::from(config.editor.page_size) as usize);
        // On first launch (no saved config): auto-detect the system dark/light mode.
        // On subsequent launches: respect the theme the user explicitly chose and saved.
        let theme = if config_file_exists {
            config.appearance.theme
        } else if CurrentPlatform::is_dark_mode_enabled() {
            Theme::Dark
        } else {
            Theme::Light
        };
        state.ui.set_theme(theme);
    }

    // Open the single shared SQLite database for all persistence needs.
    let pool = runtime.block_on(async {
        sqlx::sqlite::SqlitePoolOptions::new()
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(config_dir.join("wellfeather.db"))
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

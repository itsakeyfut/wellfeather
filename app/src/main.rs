slint::include_modules!();

mod app;
mod state;
mod ui;

use std::sync::Arc;

use app::controller::AppController;
use state::AppState;
use ui::UI;
use wf_db::service::DbService;

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

    let (controller, tx_cmd, rx_event) = AppController::new(state.clone(), db);
    tokio::spawn(controller.run());

    let ui = UI::new(state, tx_cmd, rx_event)?;
    ui.run()
}

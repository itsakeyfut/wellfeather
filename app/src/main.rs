slint::include_modules!();

mod app;
mod state;
mod ui;

use std::sync::Arc;

use state::AppState;
use ui::UI;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    // Keep the runtime context active on the main thread so tokio::spawn
    // calls from Slint callbacks work without an explicit runtime handle.
    let _guard = runtime.enter();

    let state = Arc::new(AppState::new());
    let ui = UI::new(state)?;
    ui.run()
}

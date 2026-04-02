#![allow(dead_code)]

use anyhow::Result;
use slint::ComponentHandle;

use crate::state::SharedState;

pub struct UI {
    window: crate::AppWindow,
}

impl UI {
    pub fn new(state: SharedState) -> Result<Self> {
        let window = crate::AppWindow::new()?;

        Self::register_sidebar_callbacks(&window, state.clone());
        Self::register_editor_callbacks(&window, state.clone());
        Self::register_result_callbacks(&window, state.clone());
        Self::register_status_callbacks(&window, state);

        Ok(Self { window })
    }

    pub fn run(&self) -> Result<()> {
        self.window.run()?;
        Ok(())
    }

    fn register_sidebar_callbacks(_window: &crate::AppWindow, _state: SharedState) {
        // TODO: T020+ — connection tree interaction, table double-click
    }

    fn register_editor_callbacks(_window: &crate::AppWindow, _state: SharedState) {
        // TODO: T020+ — run_query, cancel_query, completion trigger
    }

    fn register_result_callbacks(_window: &crate::AppWindow, _state: SharedState) {
        // TODO: T020+ — copy, export, virtual scroll
    }

    fn register_status_callbacks(_window: &crate::AppWindow, _state: SharedState) {
        // TODO: T020+ — status updates via invoke_from_event_loop
    }
}

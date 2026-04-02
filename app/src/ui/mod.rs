#![allow(dead_code)]

use anyhow::Result;
use slint::ComponentHandle;
use tokio::sync::mpsc;

use crate::{
    app::{command::Command, event::Event},
    state::SharedState,
};

pub struct UI {
    window: crate::AppWindow,
}

impl UI {
    pub fn new(
        state: SharedState,
        tx_cmd: mpsc::Sender<Command>,
        rx_event: mpsc::Receiver<Event>,
    ) -> Result<Self> {
        let window = crate::AppWindow::new()?;

        Self::register_sidebar_callbacks(&window, state.clone(), tx_cmd.clone());
        Self::register_editor_callbacks(&window, state.clone(), tx_cmd.clone());
        Self::register_result_callbacks(&window, state.clone());
        Self::register_status_callbacks(&window, state);
        Self::spawn_event_handler(&window, rx_event);

        Ok(Self { window })
    }

    pub fn run(&self) -> Result<()> {
        self.window.run()?;
        Ok(())
    }

    fn spawn_event_handler(window: &crate::AppWindow, mut rx_event: mpsc::Receiver<Event>) {
        let window_weak = window.as_weak();
        tokio::spawn(async move {
            while let Some(event) = rx_event.recv().await {
                // clone required: invoke_from_event_loop closure must be 'static
                let window_weak = window_weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(window) = window_weak.upgrade() else {
                        return;
                    };
                    let ui = window.global::<crate::UiState>();
                    match event {
                        Event::Connected(id) => {
                            ui.set_status_message(format!("Connected: {id}").into());
                            ui.set_error_message("".into());
                        }
                        Event::Disconnected(id) => {
                            ui.set_status_message(format!("Disconnected: {id}").into());
                        }
                        Event::QueryError(msg) => {
                            ui.set_error_message(msg.into());
                        }
                        _ => {}
                    }
                });
            }
        });
    }

    fn register_sidebar_callbacks(
        _window: &crate::AppWindow,
        _state: SharedState,
        _tx_cmd: mpsc::Sender<Command>,
    ) {
        // TODO: T027 — connection tree interaction
    }

    fn register_editor_callbacks(
        _window: &crate::AppWindow,
        _state: SharedState,
        _tx_cmd: mpsc::Sender<Command>,
    ) {
        // TODO: T030+ — run_query, cancel_query, completion trigger
    }

    fn register_result_callbacks(_window: &crate::AppWindow, _state: SharedState) {
        // TODO: T020+ — copy, export, virtual scroll
    }

    fn register_status_callbacks(_window: &crate::AppWindow, _state: SharedState) {
        // TODO: T029 — status updates via invoke_from_event_loop
    }
}

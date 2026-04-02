#![allow(dead_code)]

use std::rc::Rc;

use anyhow::Result;
use slint::ComponentHandle;
use tokio::sync::mpsc;
use wf_db::models::{DbConnection, DbType};

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
        Self::register_connection_form_callbacks(&window, tx_cmd.clone());
        Self::register_editor_callbacks(&window, state.clone(), tx_cmd.clone());
        Self::register_result_callbacks(&window, state.clone());
        Self::register_status_callbacks(&window, state.clone());
        Self::spawn_event_handler(&window, rx_event, state);

        Ok(Self { window })
    }

    pub fn run(&self) -> Result<()> {
        self.window.run()?;
        Ok(())
    }

    // ── Event handler task ────────────────────────────────────────────────────

    fn spawn_event_handler(
        window: &crate::AppWindow,
        mut rx_event: mpsc::Receiver<Event>,
        state: SharedState,
    ) {
        let window_weak = window.as_weak();
        tokio::spawn(async move {
            while let Some(event) = rx_event.recv().await {
                match event {
                    Event::Connected(ref id) => {
                        let active_id = id.clone();
                        // Build connection list from state outside invoke_from_event_loop
                        // (Vec<ConnectionEntry> is Send; Rc<VecModel> is not).
                        let entries: Vec<crate::ConnectionEntry> = state
                            .conn
                            .all()
                            .into_iter()
                            .map(|c| crate::ConnectionEntry {
                                is_active: c.id == active_id,
                                db_type: db_type_label(&c.db_type).into(),
                                name: c.name.clone().into(),
                                id: c.id.clone().into(),
                            })
                            .collect();

                        // clone required: invoke_from_event_loop closure must be 'static
                        let window_weak = window_weak.clone();
                        let active_id = active_id.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            let Some(window) = window_weak.upgrade() else {
                                return;
                            };
                            let ui = window.global::<crate::UiState>();
                            // VecModel created on the UI thread (Rc is not Send)
                            let model = Rc::new(slint::VecModel::from(entries));
                            ui.set_connection_list(model.into());
                            ui.set_active_connection_id(active_id.into());
                            ui.set_show_connection_form(false);
                            ui.set_form_testing(false);
                            ui.set_form_status("".into());
                            ui.set_error_message("".into());
                        });
                    }
                    Event::QueryError(ref msg) => {
                        let msg = msg.clone();
                        // clone required: invoke_from_event_loop closure must be 'static
                        let window_weak = window_weak.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            let Some(window) = window_weak.upgrade() else {
                                return;
                            };
                            let ui = window.global::<crate::UiState>();
                            ui.set_form_status(msg.clone().into());
                            ui.set_form_testing(false);
                            ui.set_error_message(msg.into());
                        });
                    }
                    Event::Disconnected(ref id) => {
                        let id = id.clone();
                        // clone required: invoke_from_event_loop closure must be 'static
                        let window_weak = window_weak.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            let Some(window) = window_weak.upgrade() else {
                                return;
                            };
                            let ui = window.global::<crate::UiState>();
                            ui.set_status_message(format!("Disconnected: {id}").into());
                        });
                    }
                    _ => {}
                }
            }
        });
    }

    // ── Sidebar callbacks ─────────────────────────────────────────────────────

    fn register_sidebar_callbacks(
        window: &crate::AppWindow,
        state: SharedState,
        tx_cmd: mpsc::Sender<Command>,
    ) {
        let ui_state = window.global::<crate::UiState>();

        // open-connection-form: reset form fields then show the overlay
        {
            let window_weak = window.as_weak();
            ui_state.on_open_connection_form(move || {
                let Some(window) = window_weak.upgrade() else {
                    return;
                };
                let ui = window.global::<crate::UiState>();
                ui.set_form_name("".into());
                ui.set_form_conn_string("".into());
                ui.set_form_host("".into());
                ui.set_form_port("".into());
                ui.set_form_user("".into());
                ui.set_form_password("".into());
                ui.set_form_database("".into());
                ui.set_form_status("".into());
                ui.set_form_testing(false);
                ui.set_form_tab_index(0);
                ui.set_form_db_type(0);
                ui.set_show_connection_form(true);
            });
        }

        // switch-connection: look up the saved conn by id and re-connect
        {
            // clone required: callback closure needs owned tx_cmd
            let tx_cmd = tx_cmd.clone();
            // clone required: callback closure needs owned state
            let state = state.clone();
            ui_state.on_switch_connection(move |id| {
                let id = id.to_string();
                let conn = state.conn.all().into_iter().find(|c| c.id == id);
                if let Some(conn) = conn {
                    // clone required: tokio::spawn requires 'static
                    let tx_cmd = tx_cmd.clone();
                    tokio::spawn(async move {
                        let _ = tx_cmd.send(Command::Connect(conn, None)).await;
                    });
                }
            });
        }
    }

    // ── Connection form callbacks ─────────────────────────────────────────────

    fn register_connection_form_callbacks(
        window: &crate::AppWindow,
        tx_cmd: mpsc::Sender<Command>,
    ) {
        let ui_state = window.global::<crate::UiState>();

        // close-connection-form
        {
            let window_weak = window.as_weak();
            ui_state.on_close_connection_form(move || {
                let Some(window) = window_weak.upgrade() else {
                    return;
                };
                window
                    .global::<crate::UiState>()
                    .set_show_connection_form(false);
            });
        }

        // test-connection: build DbConnection from form, send Command::Connect
        {
            let window_weak = window.as_weak();
            // clone required: callback closure needs owned tx_cmd
            let tx_cmd = tx_cmd.clone();
            ui_state.on_test_connection(move || {
                let Some(window) = window_weak.upgrade() else {
                    return;
                };
                let ui = window.global::<crate::UiState>();
                ui.set_form_testing(true);
                ui.set_form_status("".into());

                let (conn, password) = build_conn_from_form(&ui);
                // clone required: tokio::spawn requires 'static
                let tx_cmd = tx_cmd.clone();
                tokio::spawn(async move {
                    let _ = tx_cmd.send(Command::Connect(conn, password)).await;
                });
            });
        }
    }

    // ── Editor callbacks (TODO) ───────────────────────────────────────────────

    fn register_editor_callbacks(
        _window: &crate::AppWindow,
        _state: SharedState,
        _tx_cmd: mpsc::Sender<Command>,
    ) {
        // TODO: T030+ — run_query, cancel_query, completion trigger
    }

    // ── Result callbacks (TODO) ───────────────────────────────────────────────

    fn register_result_callbacks(_window: &crate::AppWindow, _state: SharedState) {
        // TODO: T020+ — copy, export, virtual scroll
    }

    // ── Status callbacks (TODO) ───────────────────────────────────────────────

    fn register_status_callbacks(_window: &crate::AppWindow, _state: SharedState) {
        // TODO: T029 — status updates via invoke_from_event_loop
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn db_type_label(dt: &DbType) -> &'static str {
    match dt {
        DbType::PostgreSQL => "PostgreSQL",
        DbType::MySQL => "MySQL",
        DbType::SQLite => "SQLite",
    }
}

/// Build a `DbConnection` from the current values in the connection form global,
/// and return the plaintext password separately (to avoid storing it on the model).
fn build_conn_from_form(ui: &crate::UiState) -> (DbConnection, Option<String>) {
    let db_type = match ui.get_form_db_type() {
        0 => DbType::PostgreSQL,
        1 => DbType::MySQL,
        _ => DbType::SQLite,
    };

    let is_conn_string = ui.get_form_tab_index() == 0;
    let opt = |s: slint::SharedString| {
        let s = s.to_string();
        if s.is_empty() { None } else { Some(s) }
    };

    let password = if is_conn_string {
        None
    } else {
        opt(ui.get_form_password())
    };

    let conn = DbConnection {
        id: uuid::Uuid::new_v4().to_string(),
        name: ui.get_form_name().to_string(),
        db_type,
        connection_string: if is_conn_string {
            opt(ui.get_form_conn_string())
        } else {
            None
        },
        host: if is_conn_string {
            None
        } else {
            opt(ui.get_form_host())
        },
        port: if is_conn_string {
            None
        } else {
            ui.get_form_port().to_string().parse::<u16>().ok()
        },
        user: if is_conn_string {
            None
        } else {
            opt(ui.get_form_user())
        },
        // Encryption is wired in T028; password flows via Command::Connect's second field.
        password_encrypted: None,
        database: if is_conn_string {
            None
        } else {
            opt(ui.get_form_database())
        },
    };

    (conn, password)
}

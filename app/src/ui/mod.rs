#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use slint::ComponentHandle;
use slint::Model as _;
use tokio::sync::mpsc;
use wf_config::crypto;
use wf_db::models::{DbConnection, DbMetadata, DbType, TableInfo};

use crate::{
    app::{command::Command, event::Event},
    state::SharedState,
};

// ---------------------------------------------------------------------------
// Sidebar tree state
// ---------------------------------------------------------------------------

#[derive(Default)]
struct SidebarUiState {
    metadata: HashMap<String, DbMetadata>,
    expanded: HashSet<String>,
}

fn build_sidebar_tree(
    connections: &[DbConnection],
    active_id: &str,
    metadata: &HashMap<String, DbMetadata>,
    expanded: &HashSet<String>,
) -> Vec<crate::SidebarNode> {
    let mut nodes = vec![];
    for conn in connections {
        let conn_node_id = format!("conn:{}", conn.id);
        let is_conn_expanded = expanded.contains(&conn_node_id);
        nodes.push(crate::SidebarNode {
            id: conn_node_id.clone().into(),
            label: conn.name.clone().into(),
            sub_label: db_type_label(&conn.db_type).into(),
            level: 0,
            is_expanded: is_conn_expanded,
            is_active: conn.id == active_id,
            node_kind: "connection".into(),
        });
        if !is_conn_expanded {
            continue;
        }
        let Some(meta) = metadata.get(&conn.id) else {
            continue;
        };
        push_tableinfo_category(
            &mut nodes,
            &conn.id,
            "Tables",
            &meta.tables,
            "table",
            expanded,
        );
        push_tableinfo_category(&mut nodes, &conn.id, "Views", &meta.views, "view", expanded);
        push_string_category(
            &mut nodes,
            &conn.id,
            "Stored Procedures",
            &meta.stored_procs,
            "proc",
            expanded,
        );
        push_string_category(
            &mut nodes,
            &conn.id,
            "Indexes",
            &meta.indexes,
            "index",
            expanded,
        );
    }
    nodes
}

fn push_tableinfo_category(
    nodes: &mut Vec<crate::SidebarNode>,
    conn_id: &str,
    name: &str,
    items: &[TableInfo],
    kind: &str,
    expanded: &HashSet<String>,
) {
    let cat_id = format!("cat:{}:{}", conn_id, name);
    let is_exp = expanded.contains(&cat_id);
    nodes.push(crate::SidebarNode {
        id: cat_id.into(),
        label: name.into(),
        sub_label: "".into(),
        level: 1,
        is_expanded: is_exp,
        is_active: false,
        node_kind: "category".into(),
    });
    if is_exp {
        for item in items {
            nodes.push(crate::SidebarNode {
                id: format!("item:{}:{}:{}", conn_id, kind, item.name).into(),
                label: item.name.clone().into(),
                sub_label: "".into(),
                level: 2,
                is_expanded: false,
                is_active: false,
                node_kind: kind.into(),
            });
        }
    }
}

fn push_string_category(
    nodes: &mut Vec<crate::SidebarNode>,
    conn_id: &str,
    name: &str,
    items: &[String],
    kind: &str,
    expanded: &HashSet<String>,
) {
    let cat_id = format!("cat:{}:{}", conn_id, name);
    let is_exp = expanded.contains(&cat_id);
    nodes.push(crate::SidebarNode {
        id: cat_id.into(),
        label: name.into(),
        sub_label: "".into(),
        level: 1,
        is_expanded: is_exp,
        is_active: false,
        node_kind: "category".into(),
    });
    if is_exp {
        for item in items {
            nodes.push(crate::SidebarNode {
                id: format!("item:{}:{}:{}", conn_id, kind, item).into(),
                label: item.clone().into(),
                sub_label: "".into(),
                level: 2,
                is_expanded: false,
                is_active: false,
                node_kind: kind.into(),
            });
        }
    }
}

// ---------------------------------------------------------------------------
// UI
// ---------------------------------------------------------------------------

pub struct UI {
    window: crate::AppWindow,
}

impl UI {
    pub fn new(
        state: SharedState,
        tx_cmd: mpsc::Sender<Command>,
        rx_event: mpsc::Receiver<Event>,
        enc_key: [u8; 32],
    ) -> Result<Self> {
        let window = crate::AppWindow::new()?;

        let sidebar_state: Arc<Mutex<SidebarUiState>> =
            Arc::new(Mutex::new(SidebarUiState::default()));

        Self::register_sidebar_callbacks(
            &window,
            state.clone(),
            tx_cmd.clone(),
            Arc::clone(&sidebar_state),
        );
        Self::register_connection_form_callbacks(&window, tx_cmd.clone(), enc_key);
        Self::register_editor_callbacks(&window, state.clone(), tx_cmd.clone());
        Self::register_result_callbacks(&window, state.clone());
        Self::register_status_callbacks(&window, state.clone());
        Self::spawn_event_handler(&window, rx_event, state, Arc::clone(&sidebar_state));

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
        sidebar_state: Arc<Mutex<SidebarUiState>>,
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

                        // Build status-bar label outside the closure (state is not Send)
                        let status_conn = state
                            .conn
                            .all()
                            .into_iter()
                            .find(|c| c.id == active_id)
                            .map(|c| match c.database.as_deref() {
                                Some(db) if !db.is_empty() => format!("{} / {}", c.name, db),
                                _ => c.name.clone(),
                            })
                            .unwrap_or_else(|| active_id.clone());

                        // Auto-expand the newly connected node
                        {
                            let mut sb = sidebar_state.lock().unwrap_or_else(|p| p.into_inner());
                            sb.expanded.insert(format!("conn:{}", active_id));
                        }
                        // Build sidebar tree (Vec<SidebarNode> is Send)
                        let sidebar_nodes = {
                            let sb = sidebar_state.lock().unwrap_or_else(|p| p.into_inner());
                            let connections = state.conn.all();
                            build_sidebar_tree(&connections, &active_id, &sb.metadata, &sb.expanded)
                        };

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
                            ui.set_status_connection(status_conn.into());
                            ui.set_sidebar_tree(
                                Rc::new(slint::VecModel::from(sidebar_nodes)).into(),
                            );
                            ui.set_sidebar_loading(true);
                        });
                    }
                    Event::TestConnectionOk => {
                        // clone required: invoke_from_event_loop closure must be 'static
                        let window_weak = window_weak.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            let Some(window) = window_weak.upgrade() else {
                                return;
                            };
                            let ui = window.global::<crate::UiState>();
                            ui.set_form_testing(false);
                            ui.set_form_test_ok(true);
                            ui.set_test_result_ok(true);
                            ui.set_test_result_message("".into());
                            ui.set_show_test_result_popup(true);
                        });
                    }
                    Event::TestConnectionFailed(ref msg) => {
                        let msg = msg.clone();
                        // clone required: invoke_from_event_loop closure must be 'static
                        let window_weak = window_weak.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            let Some(window) = window_weak.upgrade() else {
                                return;
                            };
                            let ui = window.global::<crate::UiState>();
                            ui.set_form_testing(false);
                            ui.set_form_test_ok(false);
                            ui.set_test_result_ok(false);
                            ui.set_test_result_message(msg.into());
                            ui.set_show_test_result_popup(true);
                        });
                    }
                    Event::ConnectError(ref msg) => {
                        let msg = msg.clone();
                        // clone required: invoke_from_event_loop closure must be 'static
                        let window_weak = window_weak.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            let Some(window) = window_weak.upgrade() else {
                                return;
                            };
                            let ui = window.global::<crate::UiState>();
                            ui.set_form_testing(false);
                            ui.set_form_status(msg.clone().into());
                            ui.set_status_message(format!("Connection failed: {msg}").into());
                        });
                    }
                    Event::QueryStarted => {
                        // clone required: invoke_from_event_loop closure must be 'static
                        let window_weak = window_weak.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            let Some(window) = window_weak.upgrade() else {
                                return;
                            };
                            let ui = window.global::<crate::UiState>();
                            ui.set_is_loading(true);
                            ui.set_error_message("".into());
                            ui.set_status_message("Running\u{2026}".into());
                            // Reveal the result panel if it is currently hidden.
                            ui.set_result_panel_open(true);
                        });
                    }
                    Event::QueryFinished(result) => {
                        // Build plain (Send) data outside the closure — Rc is not Send.
                        let col_count = result.columns.len();
                        let columns: Vec<slint::SharedString> =
                            result.columns.iter().map(|c| c.clone().into()).collect();
                        let raw_rows: Vec<Vec<slint::SharedString>> = result
                            .rows
                            .iter()
                            .map(|r| {
                                r.iter()
                                    .map(|cell| cell.as_deref().unwrap_or("").to_string().into())
                                    .collect()
                            })
                            .collect();
                        let row_count = result.row_count as i32;
                        let exec_ms = result.execution_time_ms;
                        // clone required: invoke_from_event_loop closure must be 'static
                        let window_weak = window_weak.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            let Some(window) = window_weak.upgrade() else {
                                return;
                            };
                            let ui = window.global::<crate::UiState>();
                            ui.set_is_loading(false);
                            // VecModel created on UI thread (Rc is not Send)
                            let col_model = Rc::new(slint::VecModel::from(columns));
                            ui.set_result_columns(col_model.into());
                            let rows: Vec<crate::RowData> = raw_rows
                                .into_iter()
                                .map(|cells| crate::RowData {
                                    cells: Rc::new(slint::VecModel::from(cells)).into(),
                                })
                                .collect();
                            ui.set_result_rows(Rc::new(slint::VecModel::from(rows)).into());
                            ui.set_result_row_count(row_count);
                            // Initialise per-column widths (150 px each).
                            const DEFAULT_COL_W: f32 = 150.0;
                            let widths: Vec<f32> = vec![DEFAULT_COL_W; col_count];
                            let total_w = col_count as f32 * DEFAULT_COL_W;
                            ui.set_result_col_widths(Rc::new(slint::VecModel::from(widths)).into());
                            ui.set_result_total_col_width(total_w);
                            ui.set_status_message(
                                format!("{exec_ms} ms  ·  {row_count} rows").into(),
                            );
                            // Reveal the result panel if it is currently hidden.
                            ui.set_result_panel_open(true);
                        });
                    }
                    Event::QueryCancelled => {
                        // clone required: invoke_from_event_loop closure must be 'static
                        let window_weak = window_weak.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            let Some(window) = window_weak.upgrade() else {
                                return;
                            };
                            let ui = window.global::<crate::UiState>();
                            ui.set_is_loading(false);
                            ui.set_status_message("Cancelled".into());
                        });
                    }
                    Event::QueryError(ref msg) => {
                        let msg = msg.clone();
                        // Short summary: first non-empty line, truncated to 80 chars.
                        let summary = msg
                            .lines()
                            .find(|l| !l.trim().is_empty())
                            .unwrap_or(&msg)
                            .chars()
                            .take(80)
                            .collect::<String>();
                        // clone required: invoke_from_event_loop closure must be 'static
                        let window_weak = window_weak.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            let Some(window) = window_weak.upgrade() else {
                                return;
                            };
                            let ui = window.global::<crate::UiState>();
                            ui.set_is_loading(false);
                            ui.set_form_status(msg.clone().into());
                            ui.set_form_testing(false);
                            ui.set_error_message(msg.into());
                            ui.set_status_message(format!("Error: {summary}").into());
                            // Reveal the result panel so the error is visible.
                            ui.set_result_panel_open(true);
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
                            ui.set_status_connection("Not connected".into());
                        });
                    }
                    Event::MetadataLoaded(ref conn_id, ref meta) => {
                        let conn_id = conn_id.clone();
                        let meta = meta.clone(); // clone required: moved into sidebar_state
                        // Store metadata in shared state
                        {
                            let mut sb = sidebar_state.lock().unwrap_or_else(|p| p.into_inner());
                            sb.metadata.insert(conn_id.clone(), meta);
                        }
                        // Build updated tree outside invoke_from_event_loop (Vec is Send)
                        let nodes = {
                            let sb = sidebar_state.lock().unwrap_or_else(|p| p.into_inner());
                            let connections = state.conn.all();
                            let active_id = state
                                .conn
                                .active()
                                .map(|c| c.id.clone())
                                .unwrap_or_default();
                            build_sidebar_tree(&connections, &active_id, &sb.metadata, &sb.expanded)
                        };
                        // clone required: invoke_from_event_loop closure must be 'static
                        let window_weak = window_weak.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            let Some(window) = window_weak.upgrade() else {
                                return;
                            };
                            let ui = window.global::<crate::UiState>();
                            ui.set_sidebar_tree(Rc::new(slint::VecModel::from(nodes)).into());
                            ui.set_sidebar_loading(false);
                        });
                    }
                    Event::MetadataFetchFailed(ref msg) => {
                        let msg = msg.clone();
                        // clone required: invoke_from_event_loop closure must be 'static
                        let window_weak = window_weak.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            let Some(window) = window_weak.upgrade() else {
                                return;
                            };
                            let ui = window.global::<crate::UiState>();
                            ui.set_sidebar_loading(false);
                            ui.set_status_message(format!("Metadata unavailable: {msg}").into());
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
        sidebar_state: Arc<Mutex<SidebarUiState>>,
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
                ui.set_form_test_ok(false);
                ui.set_show_test_result_popup(false);
                ui.set_show_add_confirm_popup(false);
                ui.set_show_connection_form(true);
            });
        }

        // toggle-sidebar-node: expand/collapse a tree node; also switches active
        // connection when a level-0 (connection) node is clicked.
        {
            // clone required: callback closure needs owned captures
            let tx_cmd = tx_cmd.clone();
            let state = state.clone();
            let sidebar_state = Arc::clone(&sidebar_state);
            let window_weak = window.as_weak();
            ui_state.on_toggle_sidebar_node(move |id| {
                let id = id.to_string();
                // If this is a connection node, send a Connect command.
                if let Some(conn_id) = id.strip_prefix("conn:") {
                    let conn = state.conn.all().into_iter().find(|c| c.id == conn_id);
                    if let Some(conn) = conn {
                        // clone required: tokio::spawn requires 'static
                        let tx_cmd = tx_cmd.clone();
                        tokio::spawn(async move {
                            let _ = tx_cmd.send(Command::Connect(conn, None)).await;
                        });
                    }
                }
                // Toggle expanded state
                {
                    let mut sb = sidebar_state.lock().unwrap_or_else(|p| p.into_inner());
                    if sb.expanded.contains(&id) {
                        sb.expanded.remove(&id);
                    } else {
                        sb.expanded.insert(id.clone());
                    }
                }
                // Rebuild and push the updated tree (already on UI thread)
                let nodes = {
                    let sb = sidebar_state.lock().unwrap_or_else(|p| p.into_inner());
                    let connections = state.conn.all();
                    let active_id = state
                        .conn
                        .active()
                        .map(|c| c.id.clone())
                        .unwrap_or_default();
                    build_sidebar_tree(&connections, &active_id, &sb.metadata, &sb.expanded)
                };
                let Some(window) = window_weak.upgrade() else {
                    return;
                };
                window
                    .global::<crate::UiState>()
                    .set_sidebar_tree(Rc::new(slint::VecModel::from(nodes)).into());
            });
        }

        // table-double-clicked: stub for T045 (insert SELECT * FROM {name})
        {
            ui_state.on_table_double_clicked(move |_name| {
                // T045: insert SELECT * FROM {name} into editor
            });
        }
    }

    // ── Connection form callbacks ─────────────────────────────────────────────

    fn register_connection_form_callbacks(
        window: &crate::AppWindow,
        tx_cmd: mpsc::Sender<Command>,
        enc_key: [u8; 32],
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

        // test-connection: probe without saving — sends Command::TestConnection
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
                ui.set_form_test_ok(false); // reset stale test state

                let (conn, password) = build_conn_from_form(&ui, &enc_key);
                // clone required: tokio::spawn requires 'static
                let tx_cmd = tx_cmd.clone();
                tokio::spawn(async move {
                    let _ = tx_cmd.send(Command::TestConnection(conn, password)).await;
                });
            });
        }

        // add-connection: persist if test passed, else show confirm popup
        {
            let window_weak = window.as_weak();
            // clone required: callback closure needs owned tx_cmd
            let tx_cmd = tx_cmd.clone();
            ui_state.on_add_connection(move || {
                let Some(window) = window_weak.upgrade() else {
                    return;
                };
                let ui = window.global::<crate::UiState>();
                if ui.get_form_test_ok() {
                    // Test was successful — add directly
                    ui.set_form_testing(true);
                    let (conn, password) = build_conn_from_form(&ui, &enc_key);
                    // clone required: tokio::spawn requires 'static
                    let tx_cmd = tx_cmd.clone();
                    tokio::spawn(async move {
                        let _ = tx_cmd.send(Command::Connect(conn, password)).await;
                    });
                } else {
                    // Not tested or failed — show confirmation first
                    ui.set_show_add_confirm_popup(true);
                }
            });
        }

        // confirm-add-connection: user chose "Yes" in confirm popup
        {
            let window_weak = window.as_weak();
            // clone required: callback closure needs owned tx_cmd
            let tx_cmd = tx_cmd.clone();
            ui_state.on_confirm_add_connection(move || {
                let Some(window) = window_weak.upgrade() else {
                    return;
                };
                let ui = window.global::<crate::UiState>();
                ui.set_show_add_confirm_popup(false);
                ui.set_form_testing(true);
                let (conn, password) = build_conn_from_form(&ui, &enc_key);
                // clone required: tokio::spawn requires 'static
                let tx_cmd = tx_cmd.clone();
                tokio::spawn(async move {
                    let _ = tx_cmd.send(Command::Connect(conn, password)).await;
                });
            });
        }

        // dismiss-test-popup: close the test-result popup
        {
            let window_weak = window.as_weak();
            ui_state.on_dismiss_test_popup(move || {
                let Some(window) = window_weak.upgrade() else {
                    return;
                };
                window
                    .global::<crate::UiState>()
                    .set_show_test_result_popup(false);
            });
        }

        // dismiss-add-confirm: user chose "No" in confirm popup
        {
            let window_weak = window.as_weak();
            ui_state.on_dismiss_add_confirm(move || {
                let Some(window) = window_weak.upgrade() else {
                    return;
                };
                window
                    .global::<crate::UiState>()
                    .set_show_add_confirm_popup(false);
            });
        }
    }

    // ── Editor callbacks (TODO) ───────────────────────────────────────────────

    fn register_editor_callbacks(
        window: &crate::AppWindow,
        _state: SharedState,
        tx_cmd: mpsc::Sender<Command>,
    ) {
        let ui = window.global::<crate::UiState>();

        // Pure callback: count newlines + 1 to derive the line count for the
        // line-number gutter. Declared `pure` so Slint can call it inside a
        // property binding expression (UiState.count-lines(UiState.editor-text)).
        ui.on_count_lines(|text| (text.chars().filter(|&c| c == '\n').count() + 1) as i32);

        // Pure callback: count newlines before the cursor byte offset to get
        // the 0-based line index for the current-line highlight.
        ui.on_cursor_line(|text, pos| {
            let pos = (pos as usize).min(text.as_str().len());
            text.as_str().as_bytes()[..pos]
                .iter()
                .filter(|&&b| b == b'\n')
                .count() as i32
        });

        // Pure callback: move cursor by `delta` lines (-1=up, +1=down) from
        // byte offset `pos`, preserving column position.  Returns new byte offset.
        ui.on_move_cursor_line(|text, pos, delta| {
            let s = text.as_str();
            let pos = (pos as usize).min(s.len());

            // Byte offset of the start of the current line.
            let line_start = s[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
            // Column as byte count from line start (preserved when moving).
            let col = pos - line_start;

            if delta < 0 {
                // Move up: target the previous line.
                if line_start == 0 {
                    return 0; // Already on first line — go to start.
                }
                let prev_end = line_start - 1; // byte index of the \n before us
                let prev_start = s[..prev_end].rfind('\n').map(|i| i + 1).unwrap_or(0);
                (prev_start + col.min(prev_end - prev_start)) as i32
            } else {
                // Move down: target the next line.
                match s[pos..].find('\n') {
                    None => pos as i32, // Already on last line — stay.
                    Some(off) => {
                        let next_start = pos + off + 1;
                        let next_end = s[next_start..]
                            .find('\n')
                            .map(|i| next_start + i)
                            .unwrap_or(s.len());
                        (next_start + col.min(next_end - next_start)) as i32
                    }
                }
            }
        });

        {
            let tx_cmd = tx_cmd.clone(); // clone required: callback closure needs owned tx_cmd
            ui.on_run_query(move |sql| {
                let sql = sql.to_string();
                let tx_cmd = tx_cmd.clone(); // clone required: tokio::spawn requires 'static
                tokio::spawn(async move {
                    let _ = tx_cmd.send(Command::RunQuery(sql)).await;
                });
            });
        }
        {
            let tx_cmd = tx_cmd.clone(); // clone required: callback closure needs owned tx_cmd
            ui.on_cancel_query(move || {
                let tx_cmd = tx_cmd.clone(); // clone required: tokio::spawn requires 'static
                tokio::spawn(async move {
                    let _ = tx_cmd.send(Command::CancelQuery).await;
                });
            });
        }
    }

    // ── Result callbacks ──────────────────────────────────────────────────────

    fn register_result_callbacks(window: &crate::AppWindow, _state: SharedState) {
        let ui_state = window.global::<crate::UiState>();
        // clone required: callback closure must be 'static
        let window_weak = window.as_weak();

        // resize-result-column: update the column width VecModel in place and
        // recompute the total so viewport-width stays accurate during drag.
        ui_state.on_resize_result_column(move |i, w| {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            let ui = window.global::<crate::UiState>();
            let model = ui.get_result_col_widths();
            let n = model.row_count();
            if (i as usize) < n {
                model.set_row_data(i as usize, w);
                let total: f32 = (0..n).filter_map(|j| model.row_data(j)).sum();
                ui.set_result_total_col_width(total);
            }
        });
    }

    // ── Status callbacks (TODO) ───────────────────────────────────────────────

    fn register_status_callbacks(_window: &crate::AppWindow, _state: SharedState) {
        // Status bar text is updated by spawn_event_handler via invoke_from_event_loop.
        // No additional setup needed here.
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
/// and return the plaintext password separately (for immediate use in the connection URL).
///
/// The plaintext password is also AES-256-GCM encrypted with `enc_key` and stored in
/// `DbConnection.password_encrypted` so the session manager can persist it and
/// `main.rs` can decrypt it on the next startup for auto-reconnect.
fn build_conn_from_form(ui: &crate::UiState, enc_key: &[u8; 32]) -> (DbConnection, Option<String>) {
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

    // Encrypt the plaintext password for safe storage in config.toml.
    // Connection-string mode embeds the password in the URL, so no separate encryption needed.
    let password_encrypted = password.as_ref().map(|pw| crypto::encrypt(pw, enc_key));

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
        password_encrypted,
        database: if is_conn_string {
            None
        } else {
            opt(ui.get_form_database())
        },
    };

    (conn, password)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use wf_db::models::{DbMetadata, DbType, TableInfo};

    fn make_conn(id: &str, name: &str) -> DbConnection {
        DbConnection {
            id: id.to_string(),
            name: name.to_string(),
            db_type: DbType::SQLite,
            connection_string: None,
            host: None,
            port: None,
            user: None,
            password_encrypted: None,
            database: None,
        }
    }

    fn make_meta(tables: &[&str]) -> DbMetadata {
        DbMetadata {
            tables: tables
                .iter()
                .map(|n| TableInfo {
                    name: n.to_string(),
                    columns: vec![],
                })
                .collect(),
            views: vec![],
            stored_procs: vec![],
            indexes: vec![],
        }
    }

    #[test]
    fn build_sidebar_tree_should_render_connection_nodes() {
        let conns = vec![make_conn("a", "Alpha"), make_conn("b", "Beta")];
        let nodes = build_sidebar_tree(&conns, "", &HashMap::new(), &HashSet::new());
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].label.as_str(), "Alpha");
        assert_eq!(nodes[0].level, 0);
        assert_eq!(nodes[0].node_kind.as_str(), "connection");
        assert_eq!(nodes[1].label.as_str(), "Beta");
    }

    #[test]
    fn build_sidebar_tree_should_show_categories_when_connection_expanded() {
        let conns = vec![make_conn("a", "Alpha")];
        let mut expanded = HashSet::new();
        expanded.insert("conn:a".to_string());
        let mut metadata = HashMap::new();
        metadata.insert("a".to_string(), make_meta(&["users"]));
        let nodes = build_sidebar_tree(&conns, "a", &metadata, &expanded);
        // conn + Tables + Views + Stored Procedures + Indexes = 5 nodes
        assert_eq!(nodes.len(), 5);
        assert_eq!(nodes[1].label.as_str(), "Tables");
        assert_eq!(nodes[1].level, 1);
        assert_eq!(nodes[1].node_kind.as_str(), "category");
    }

    #[test]
    fn build_sidebar_tree_should_show_items_when_category_expanded() {
        let conns = vec![make_conn("a", "Alpha")];
        let mut expanded = HashSet::new();
        expanded.insert("conn:a".to_string());
        expanded.insert("cat:a:Tables".to_string());
        let mut metadata = HashMap::new();
        metadata.insert("a".to_string(), make_meta(&["users", "orders"]));
        let nodes = build_sidebar_tree(&conns, "a", &metadata, &expanded);
        // conn + Tables + users + orders + Views + Stored Procedures + Indexes = 7
        assert_eq!(nodes.len(), 7);
        assert_eq!(nodes[2].label.as_str(), "users");
        assert_eq!(nodes[2].level, 2);
        assert_eq!(nodes[2].node_kind.as_str(), "table");
        assert_eq!(nodes[3].label.as_str(), "orders");
    }

    #[test]
    fn build_sidebar_tree_should_hide_children_when_collapsed() {
        let conns = vec![make_conn("a", "Alpha")];
        let nodes = build_sidebar_tree(&conns, "a", &HashMap::new(), &HashSet::new());
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].level, 0);
    }

    #[test]
    fn build_sidebar_tree_should_mark_active_connection() {
        let conns = vec![make_conn("a", "Alpha"), make_conn("b", "Beta")];
        let nodes = build_sidebar_tree(&conns, "b", &HashMap::new(), &HashSet::new());
        assert!(!nodes[0].is_active);
        assert!(nodes[1].is_active);
    }
}

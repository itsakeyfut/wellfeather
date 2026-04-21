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

// ---------------------------------------------------------------------------
// Original query result — retained for client-side filtering
// ---------------------------------------------------------------------------

struct OriginalQueryData {
    columns: Vec<slint::SharedString>,
    // None = SQL NULL; Some(s) = value (including empty string)
    rows: Vec<Vec<Option<String>>>,
    /// None = unsorted; Some(i) = sort column index.
    sort_col: Option<usize>,
    sort_asc: bool,
}

type SharedOriginalData = Arc<Mutex<Option<OriginalQueryData>>>;

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
        let conn_idx = nodes.len() as i32;
        nodes.push(crate::SidebarNode {
            id: conn_node_id.clone().into(),
            label: conn.name.clone().into(),
            sub_label: db_type_label(&conn.db_type).into(),
            level: 0,
            is_expanded: is_conn_expanded,
            is_active: conn.id == active_id,
            node_kind: "connection".into(),
            parent_index: -1,
        });
        if !is_conn_expanded {
            continue;
        }
        let Some(meta) = metadata.get(&conn.id) else {
            continue;
        };
        push_tableinfo_category(
            &mut nodes,
            conn_idx,
            &conn.id,
            "Tables",
            &meta.tables,
            "table",
            expanded,
        );
        push_tableinfo_category(
            &mut nodes,
            conn_idx,
            &conn.id,
            "Views",
            &meta.views,
            "view",
            expanded,
        );
        push_string_category(
            &mut nodes,
            conn_idx,
            &conn.id,
            "Stored Procedures",
            &meta.stored_procs,
            "proc",
            expanded,
        );
        push_string_category(
            &mut nodes,
            conn_idx,
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
    conn_idx: i32,
    conn_id: &str,
    name: &str,
    items: &[TableInfo],
    kind: &str,
    expanded: &HashSet<String>,
) {
    let cat_id = format!("cat:{}:{}", conn_id, name);
    let is_exp = expanded.contains(&cat_id);
    let cat_idx = nodes.len() as i32;
    nodes.push(crate::SidebarNode {
        id: cat_id.into(),
        label: name.into(),
        sub_label: "".into(),
        level: 1,
        is_expanded: is_exp,
        is_active: false,
        node_kind: "category".into(),
        parent_index: conn_idx,
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
                parent_index: cat_idx,
            });
        }
    }
}

fn push_string_category(
    nodes: &mut Vec<crate::SidebarNode>,
    conn_idx: i32,
    conn_id: &str,
    name: &str,
    items: &[String],
    kind: &str,
    expanded: &HashSet<String>,
) {
    let cat_id = format!("cat:{}:{}", conn_id, name);
    let is_exp = expanded.contains(&cat_id);
    let cat_idx = nodes.len() as i32;
    nodes.push(crate::SidebarNode {
        id: cat_id.into(),
        label: name.into(),
        sub_label: "".into(),
        level: 1,
        is_expanded: is_exp,
        is_active: false,
        node_kind: "category".into(),
        parent_index: conn_idx,
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
                parent_index: cat_idx,
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

        // Shared storage for the unfiltered query result; written by the event
        // handler on QueryFinished, read by the filter callbacks on the UI thread.
        let original_data: SharedOriginalData = Arc::new(Mutex::new(None));

        Self::register_sidebar_callbacks(
            &window,
            state.clone(),
            tx_cmd.clone(),
            Arc::clone(&sidebar_state),
            enc_key,
        );
        Self::register_connection_form_callbacks(&window, tx_cmd.clone(), enc_key);
        Self::register_editor_callbacks(&window, state.clone(), tx_cmd.clone());
        Self::register_result_callbacks(&window, state.clone(), Arc::clone(&original_data));
        Self::register_status_callbacks(&window, state.clone());
        Self::spawn_event_handler(
            &window,
            rx_event,
            state,
            Arc::clone(&sidebar_state),
            Arc::clone(&original_data),
        );

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
        original_data: SharedOriginalData,
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
                            ui.set_sidebar_loading(false);
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
                        // Preserve None (SQL NULL) so the badge renderer can distinguish it
                        // from an empty string.  Moved into OriginalQueryData for filtering.
                        let raw_rows: Vec<Vec<Option<String>>> =
                            result.rows.iter().map(|r| r.to_vec()).collect();
                        let row_count = result.row_count as i32;
                        let exec_ms = result.execution_time_ms;

                        // Store original rows for client-side filtering/sorting.
                        {
                            let mut orig = original_data.lock().unwrap_or_else(|p| p.into_inner());
                            *orig = Some(OriginalQueryData {
                                columns: columns.clone(),
                                rows: raw_rows.clone(),
                                sort_col: None,
                                sort_asc: true,
                            });
                        }

                        // clone required: invoke_from_event_loop closure must be 'static
                        let window_weak = window_weak.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            let Some(window) = window_weak.upgrade() else {
                                return;
                            };
                            let ui = window.global::<crate::UiState>();
                            ui.set_is_loading(false);
                            ui.set_result_active_filter("".into()); // clear stale filter
                            ui.set_result_sort_col(-1);
                            ui.set_result_sort_asc(true);
                            // VecModel created on UI thread (Rc is not Send)
                            let col_model = Rc::new(slint::VecModel::from(columns));
                            ui.set_result_columns(col_model.into());
                            let rows: Vec<crate::RowData> =
                                raw_rows.into_iter().map(rows_to_ui).collect();
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
                    Event::InsertText(ref text) => {
                        let text = text.clone();
                        // clone required: invoke_from_event_loop closure must be 'static
                        let window_weak = window_weak.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            let Some(window) = window_weak.upgrade() else {
                                return;
                            };
                            let ui = window.global::<crate::UiState>();
                            let current = ui.get_editor_text().to_string();
                            ui.set_editor_text(append_editor_text(&current, &text).into());
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
        enc_key: [u8; 32],
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
                        let password = conn
                            .password_encrypted
                            .as_ref()
                            .and_then(|enc| crypto::decrypt(enc, &enc_key).ok());
                        // clone required: tokio::spawn requires 'static
                        let tx_cmd = tx_cmd.clone();
                        tokio::spawn(async move {
                            let _ = tx_cmd.send(Command::Connect(conn, password)).await;
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

        // table-double-clicked: insert SELECT * FROM <name> into the editor
        // and immediately execute it so the result appears without a manual
        // Ctrl+Enter.  tx_cmd is cloned here because the closure is 'static.
        {
            let window_weak = window.as_weak();
            let tx_cmd = tx_cmd.clone(); // clone required: callback closure needs owned tx_cmd
            ui_state.on_table_double_clicked(move |name| {
                let sql = format!("SELECT * FROM {}", name);
                let Some(window) = window_weak.upgrade() else {
                    return;
                };
                let ui = window.global::<crate::UiState>();
                let current = ui.get_editor_text().to_string();
                ui.set_editor_text(append_editor_text(&current, &sql).into());
                // Auto-execute so the result is visible immediately.
                let tx_cmd = tx_cmd.clone(); // clone required: tokio::spawn requires 'static
                let sql_run = sql.clone();
                tokio::spawn(async move {
                    let _ = tx_cmd.send(Command::RunQuery(sql_run)).await;
                });
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

    fn register_result_callbacks(
        window: &crate::AppWindow,
        _state: SharedState,
        original_data: SharedOriginalData,
    ) {
        let ui_state = window.global::<crate::UiState>();
        let window_weak = window.as_weak();

        // resize-result-column: update the column width VecModel in place and
        // recompute the total so viewport-width stays accurate during drag.
        {
            // clone required: callback closure must be 'static
            let window_weak = window_weak.clone();
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

        // filter-result-rows: apply client-side predicate, then re-apply active sort.
        {
            // clone required: callback closure must be 'static
            let window_weak = window_weak.clone();
            let original_data = Arc::clone(&original_data);
            ui_state.on_filter_result_rows(move |query| {
                let Some(window) = window_weak.upgrade() else {
                    return;
                };
                let ui = window.global::<crate::UiState>();
                let orig = original_data.lock().unwrap_or_else(|p| p.into_inner());
                let Some(ref data) = *orig else {
                    return;
                };
                let mut filtered = filter_rows(&data.columns, &data.rows, query.as_str());
                if let Some(col) = data.sort_col {
                    sort_rows(&mut filtered, col, data.sort_asc);
                }
                let row_count = filtered.len() as i32;
                let rows: Vec<crate::RowData> = filtered.into_iter().map(rows_to_ui).collect();
                ui.set_result_rows(Rc::new(slint::VecModel::from(rows)).into());
                ui.set_result_row_count(row_count);
                ui.set_result_active_filter(query);
            });
        }

        // clear-result-filter: restore the unfiltered original rows, then re-apply active sort.
        {
            // clone required: callback closure must be 'static
            let window_weak = window_weak.clone();
            let original_data = Arc::clone(&original_data);
            ui_state.on_clear_result_filter(move || {
                let Some(window) = window_weak.upgrade() else {
                    return;
                };
                let ui = window.global::<crate::UiState>();
                let orig = original_data.lock().unwrap_or_else(|p| p.into_inner());
                let Some(ref data) = *orig else {
                    return;
                };
                let mut rows: Vec<Vec<Option<String>>> = data.rows.clone();
                if let Some(col) = data.sort_col {
                    sort_rows(&mut rows, col, data.sort_asc);
                }
                let row_count = rows.len() as i32;
                let ui_rows: Vec<crate::RowData> = rows.into_iter().map(rows_to_ui).collect();
                ui.set_result_rows(Rc::new(slint::VecModel::from(ui_rows)).into());
                ui.set_result_row_count(row_count);
                ui.set_result_active_filter("".into());
            });
        }

        // copy-result-cell: write the value to the system clipboard via arboard.
        {
            ui_state.on_copy_result_cell(move |value| {
                if let Ok(mut clip) = arboard::Clipboard::new() {
                    let _ = clip.set_text(value.as_str());
                }
            });
        }

        // col-x-offset (pure): cumulative x-position of column j (sum of widths 0..j).
        // Used by result_table.slint's `changed selected-col` handler to auto-scroll.
        {
            // clone required: callback closure must be 'static
            let window_weak = window_weak.clone();
            ui_state.on_col_x_offset(move |j| {
                let Some(window) = window_weak.upgrade() else {
                    return 0.0;
                };
                let ui = window.global::<crate::UiState>();
                let model = ui.get_result_col_widths();
                (0..j as usize).filter_map(|i| model.row_data(i)).sum()
            });
        }

        // sort-result-col: toggle sort state and re-render with filter + sort applied.
        {
            // clone required: callback closure must be 'static
            let window_weak = window_weak.clone();
            let original_data = Arc::clone(&original_data);
            ui_state.on_sort_result_col(move |col_i| {
                let Some(window) = window_weak.upgrade() else {
                    return;
                };
                let ui = window.global::<crate::UiState>();
                // Read active filter before taking the lock.
                let filter_q = ui.get_result_active_filter().to_string();
                let (new_col, new_asc, mut rows) = {
                    let mut orig = original_data.lock().unwrap_or_else(|p| p.into_inner());
                    let Some(ref mut data) = *orig else {
                        return;
                    };
                    let col = col_i as usize;
                    let (new_col, new_asc) = if data.sort_col == Some(col) {
                        (Some(col), !data.sort_asc)
                    } else {
                        (Some(col), true)
                    };
                    data.sort_col = new_col;
                    data.sort_asc = new_asc;
                    let filtered = filter_rows(&data.columns, &data.rows, &filter_q);
                    (new_col, new_asc, filtered)
                };
                if let Some(col) = new_col {
                    sort_rows(&mut rows, col, new_asc);
                }
                let row_count = rows.len() as i32;
                let ui_rows: Vec<crate::RowData> = rows.into_iter().map(rows_to_ui).collect();
                ui.set_result_rows(Rc::new(slint::VecModel::from(ui_rows)).into());
                ui.set_result_row_count(row_count);
                ui.set_result_sort_col(new_col.map(|c| c as i32).unwrap_or(-1));
                ui.set_result_sort_asc(new_asc);
            });
        }
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

/// Append `text` to `current` editor content with a newline separator.
/// If `current` is empty the text is used as-is.
/// If `current` already ends with `\n` the text is appended directly.
fn append_editor_text(current: &str, text: &str) -> String {
    if current.is_empty() {
        text.to_string()
    } else if current.ends_with('\n') {
        format!("{}{}", current, text)
    } else {
        format!("{}\n{}", current, text)
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

// ── Result table helpers ──────────────────────────────────────────────────────

/// Convert one raw result row (`Option<String>` cells) into a Slint `RowData`.
/// `None` → `RowCellData { value: "", is_null: true }`
/// `Some(s)` → `RowCellData { value: s, is_null: false }`
fn rows_to_ui(cells: Vec<Option<String>>) -> crate::RowData {
    let cell_data: Vec<crate::RowCellData> = cells
        .into_iter()
        .map(|c| crate::RowCellData {
            value: c.as_deref().unwrap_or("").into(),
            is_null: c.is_none(),
        })
        .collect();
    crate::RowData {
        cells: Rc::new(slint::VecModel::from(cell_data)).into(),
    }
}

/// Sort `rows` in-place by column `col`.
/// - Tries numeric (`f64`) comparison first; falls back to lexicographic.
/// - `None` (SQL NULL) always sorts last regardless of direction.
fn sort_rows(rows: &mut [Vec<Option<String>>], col: usize, ascending: bool) {
    rows.sort_by(|a, b| {
        let av = a.get(col).and_then(|v| v.as_deref());
        let bv = b.get(col).and_then(|v| v.as_deref());
        match (av, bv) {
            // NULL always sorts last regardless of direction.
            (None, None) => std::cmp::Ordering::Equal,
            (None, _) => std::cmp::Ordering::Greater,
            (_, None) => std::cmp::Ordering::Less,
            (Some(a), Some(b)) => {
                let ord = match (a.parse::<f64>(), b.parse::<f64>()) {
                    (Ok(af), Ok(bf)) => af.partial_cmp(&bf).unwrap_or(std::cmp::Ordering::Equal),
                    _ => a.cmp(b),
                };
                if ascending { ord } else { ord.reverse() }
            }
        }
    });
}

/// Filter `rows` according to `query`:
///
/// * Empty query → return all rows.
/// * `col_name = 'value'` → exact match on the named column (case-insensitive column name).
///   NULL cells never match an `= 'value'` predicate.
/// * Anything else → case-insensitive substring match across all columns
///   (NULL cells are treated as empty string for substring matching).
fn filter_rows(
    columns: &[slint::SharedString],
    rows: &[Vec<Option<String>>],
    query: &str,
) -> Vec<Vec<Option<String>>> {
    let query = query.trim();
    if query.is_empty() {
        return rows.to_vec();
    }
    if let Some((col_name, value)) = parse_col_eq(query) {
        let col_idx = columns
            .iter()
            .position(|c| c.as_str().eq_ignore_ascii_case(&col_name));
        match col_idx {
            Some(idx) => rows
                .iter()
                .filter(|row| row.get(idx).is_some_and(|v| v.as_deref() == Some(value)))
                .cloned()
                .collect(),
            None => vec![],
        }
    } else {
        let query_lower = query.to_lowercase();
        rows.iter()
            .filter(|row| {
                row.iter().any(|cell| {
                    cell.as_deref()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&query_lower)
                })
            })
            .cloned()
            .collect()
    }
}

/// Parse `col = 'value'` syntax.  Returns `(column_name, value_str)` on success.
fn parse_col_eq(query: &str) -> Option<(String, &str)> {
    let mut parts = query.splitn(2, '=');
    let col = parts.next()?.trim();
    let rest = parts.next()?.trim();
    let val = rest.strip_prefix('\'')?.strip_suffix('\'')?;
    Some((col.to_string(), val))
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

    // ── filter_rows tests ─────────────────────────────────────────────────────

    fn ss(s: &str) -> slint::SharedString {
        s.into()
    }

    fn sv(s: &str) -> Option<String> {
        Some(s.to_string())
    }

    #[test]
    fn filter_rows_should_return_all_when_query_empty() {
        let cols = vec![ss("id"), ss("name")];
        let rows = vec![vec![sv("1"), sv("Alice")], vec![sv("2"), sv("Bob")]];
        assert_eq!(filter_rows(&cols, &rows, "").len(), 2);
        assert_eq!(filter_rows(&cols, &rows, "   ").len(), 2);
    }

    #[test]
    fn filter_rows_should_match_substring_across_all_columns() {
        let cols = vec![ss("name"), ss("city")];
        let rows = vec![
            vec![sv("Alice"), sv("Tokyo")],
            vec![sv("Bob"), sv("Osaka")],
            vec![sv("Alice Smith"), sv("Kyoto")],
        ];
        let result = filter_rows(&cols, &rows, "alice");
        assert_eq!(result.len(), 2);
        assert_eq!(result[0][0].as_deref(), Some("Alice"));
        assert_eq!(result[1][0].as_deref(), Some("Alice Smith"));
    }

    #[test]
    fn filter_rows_should_match_exact_column_value() {
        let cols = vec![ss("name"), ss("city")];
        let rows = vec![vec![sv("Alice"), sv("Tokyo")], vec![sv("Bob"), sv("Osaka")]];
        let result = filter_rows(&cols, &rows, "city = 'Tokyo'");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0][1].as_deref(), Some("Tokyo"));
    }

    #[test]
    fn filter_rows_should_return_empty_when_column_not_found() {
        let cols = vec![ss("name")];
        let rows = vec![vec![sv("Alice")]];
        let result = filter_rows(&cols, &rows, "missing = 'x'");
        assert!(result.is_empty());
    }

    #[test]
    fn filter_rows_should_not_match_null_with_eq_predicate() {
        let cols = vec![ss("name")];
        let rows = vec![vec![None], vec![sv("Alice")]];
        let result = filter_rows(&cols, &rows, "name = ''");
        // NULL != '' — only the non-null empty string row should match, but here
        // there is none, so result is empty.
        assert!(result.is_empty());
    }

    #[test]
    fn filter_rows_should_treat_null_as_empty_for_substring_match() {
        let cols = vec![ss("name")];
        // NULL treated as "" for substring search — empty query prefix matches all.
        let rows = vec![vec![None], vec![sv("Alice")]];
        // Substring "" matches everything (but we trim, so empty query returns all).
        let result = filter_rows(&cols, &rows, "Alice");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0][0].as_deref(), Some("Alice"));
    }

    // ── sort_rows tests ───────────────────────────────────────────────────────

    #[test]
    fn sort_rows_should_sort_strings_ascending() {
        let mut rows = vec![vec![sv("banana")], vec![sv("apple")], vec![sv("cherry")]];
        sort_rows(&mut rows, 0, true);
        assert_eq!(rows[0][0].as_deref(), Some("apple"));
        assert_eq!(rows[1][0].as_deref(), Some("banana"));
        assert_eq!(rows[2][0].as_deref(), Some("cherry"));
    }

    #[test]
    fn sort_rows_should_sort_strings_descending() {
        let mut rows = vec![vec![sv("banana")], vec![sv("apple")], vec![sv("cherry")]];
        sort_rows(&mut rows, 0, false);
        assert_eq!(rows[0][0].as_deref(), Some("cherry"));
        assert_eq!(rows[1][0].as_deref(), Some("banana"));
        assert_eq!(rows[2][0].as_deref(), Some("apple"));
    }

    #[test]
    fn sort_rows_should_sort_numerically_when_values_are_numbers() {
        let mut rows = vec![vec![sv("10")], vec![sv("2")], vec![sv("20")]];
        sort_rows(&mut rows, 0, true);
        assert_eq!(rows[0][0].as_deref(), Some("2"));
        assert_eq!(rows[1][0].as_deref(), Some("10"));
        assert_eq!(rows[2][0].as_deref(), Some("20"));
    }

    #[test]
    fn sort_rows_should_put_nulls_last_ascending() {
        let mut rows = vec![vec![None], vec![sv("b")], vec![sv("a")]];
        sort_rows(&mut rows, 0, true);
        assert_eq!(rows[0][0].as_deref(), Some("a"));
        assert_eq!(rows[1][0].as_deref(), Some("b"));
        assert!(rows[2][0].is_none());
    }

    #[test]
    fn sort_rows_should_put_nulls_last_descending() {
        let mut rows = vec![vec![None], vec![sv("b")], vec![sv("a")]];
        sort_rows(&mut rows, 0, false);
        assert_eq!(rows[0][0].as_deref(), Some("b"));
        assert_eq!(rows[1][0].as_deref(), Some("a"));
        assert!(rows[2][0].is_none());
    }

    // ── append_editor_text tests ──────────────────────────────────────────────

    #[test]
    fn append_editor_text_should_set_text_when_editor_is_empty() {
        assert_eq!(append_editor_text("", "SELECT * FROM t"), "SELECT * FROM t");
    }

    #[test]
    fn append_editor_text_should_prepend_newline_when_content_exists() {
        assert_eq!(
            append_editor_text("SELECT 1", "SELECT * FROM t"),
            "SELECT 1\nSELECT * FROM t"
        );
    }

    #[test]
    fn append_editor_text_should_not_double_newline_when_content_ends_with_newline() {
        assert_eq!(
            append_editor_text("SELECT 1\n", "SELECT * FROM t"),
            "SELECT 1\nSELECT * FROM t"
        );
    }
}

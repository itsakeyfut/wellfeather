#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use slint::ComponentHandle;
use slint::Model as _;
use tokio::sync::mpsc;
use wf_completion::CompletionItem;
use wf_config::crypto;
use wf_db::models::{DbConnection, DbMetadata, DbType, QueryResult, TableInfo};

const COMPLETION_DEBOUNCE_MS: u64 = 300;
const ERROR_TRUNCATION_CHARS: usize = 80;
const DEFAULT_COLUMN_WIDTH: f32 = 150.0;

// ── UI-thread helpers ────────────────────────────────────────────────────────

/// Upgrade `weak`, run `f` against the UiState global; no-op if window is gone.
fn with_ui<F: FnOnce(&crate::UiState)>(weak: &slint::Weak<crate::AppWindow>, f: F) {
    let Some(window) = weak.upgrade() else {
        return;
    };
    let ui = window.global::<crate::UiState>();
    f(&ui);
}

/// Fire-and-forget: send `cmd` on `tx` from a new tokio task.
fn send_cmd(tx: &mpsc::Sender<Command>, cmd: Command) {
    let tx = tx.clone(); // clone required: tokio::spawn needs 'static
    tokio::spawn(async move {
        let _ = tx.send(cmd).await;
    });
}

/// Post a status-bar update to the UI thread from any thread.
fn set_status(weak: slint::Weak<crate::AppWindow>, msg: String) {
    let _ = slint::invoke_from_event_loop(move || {
        with_ui(&weak, |ui| ui.set_status_message(msg.into()));
    });
}

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

use wf_config::models::Theme;

use crate::{
    app::{
        command::{Command, ConfigUpdate},
        event::{Event, StateEvent},
    },
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
        Self::register_completion_callbacks(&window, tx_cmd.clone());
        Self::register_completion_accept_callback(&window);
        Self::register_formatter_callback(&window);
        Self::register_export_callbacks(&window, Arc::clone(&original_data));
        Self::register_theme_callback(&window, state.clone(), tx_cmd.clone());
        // Set initial page size and theme on the Slint window from shared state.
        window
            .global::<crate::UiState>()
            .set_page_size(state.ui.page_size() as i32);
        window
            .global::<crate::UiState>()
            .set_is_dark(state.ui.theme() == Theme::Dark);

        Self::register_result_callbacks(
            &window,
            state.clone(),
            Arc::clone(&original_data),
            tx_cmd.clone(),
        );
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
                    Event::Connected(id) => Self::handle_connected(
                        id,
                        window_weak.clone(),
                        state.clone(),
                        Arc::clone(&sidebar_state),
                    ),
                    Event::TestConnectionOk => Self::handle_test_ok(window_weak.clone()),
                    Event::TestConnectionFailed(msg) => {
                        Self::handle_test_failed(msg, window_weak.clone())
                    }
                    Event::ConnectError(msg) => {
                        Self::handle_connect_error(msg, window_weak.clone())
                    }
                    Event::QueryStarted => Self::handle_query_started(window_weak.clone()),
                    Event::QueryFinished(result) => Self::handle_query_finished(
                        result,
                        window_weak.clone(),
                        Arc::clone(&original_data),
                    ),
                    Event::QueryCancelled => Self::handle_query_cancelled(window_weak.clone()),
                    Event::QueryError(msg) => Self::handle_query_error(msg, window_weak.clone()),
                    Event::Disconnected(id) => Self::handle_disconnected(id, window_weak.clone()),
                    Event::MetadataLoaded(conn_id, meta) => Self::handle_metadata_loaded(
                        conn_id,
                        meta,
                        window_weak.clone(),
                        state.clone(),
                        Arc::clone(&sidebar_state),
                    ),
                    Event::MetadataFetchFailed(msg) => {
                        Self::handle_metadata_fetch_failed(msg, window_weak.clone())
                    }
                    Event::InsertText(text) => Self::handle_insert_text(text, window_weak.clone()),
                    Event::CompletionReady(items) => {
                        Self::handle_completion_ready(items, window_weak.clone())
                    }
                    Event::StateChanged(StateEvent::ThemeChanged(t)) => {
                        Self::handle_theme_changed(t, window_weak.clone())
                    }
                    _ => {}
                }
            }
        });
    }

    // ── Per-event handlers ─────────────────────────────────────────────────────

    fn handle_connected(
        id: String,
        ww: slint::Weak<crate::AppWindow>,
        state: SharedState,
        sidebar_state: Arc<Mutex<SidebarUiState>>,
    ) {
        // Build Send data outside invoke_from_event_loop (Rc<VecModel> is not Send).
        let entries: Vec<crate::ConnectionEntry> = state
            .conn
            .all()
            .into_iter()
            .map(|c| crate::ConnectionEntry {
                is_active: c.id == id,
                db_type: db_type_label(&c.db_type).into(),
                name: c.name.clone().into(),
                id: c.id.clone().into(),
            })
            .collect();
        let status_conn = state
            .conn
            .all()
            .into_iter()
            .find(|c| c.id == id)
            .map(|c| match c.database.as_deref() {
                Some(db) if !db.is_empty() => format!("{} / {}", c.name, db),
                _ => c.name.clone(),
            })
            .unwrap_or_else(|| id.clone());
        {
            let mut sb = sidebar_state.lock().unwrap_or_else(|p| p.into_inner());
            sb.expanded.insert(format!("conn:{}", id));
        }
        let sidebar_nodes = {
            let sb = sidebar_state.lock().unwrap_or_else(|p| p.into_inner());
            let connections = state.conn.all();
            build_sidebar_tree(&connections, &id, &sb.metadata, &sb.expanded)
        };
        // clone required: invoke_from_event_loop closure must be 'static
        let _ = slint::invoke_from_event_loop(move || {
            with_ui(&ww, move |ui| {
                let model = Rc::new(slint::VecModel::from(entries));
                ui.set_connection_list(model.into());
                ui.set_active_connection_id(id.into());
                ui.set_show_connection_form(false);
                ui.set_form_testing(false);
                ui.set_form_status("".into());
                ui.set_error_message("".into());
                ui.set_status_connection(status_conn.into());
                ui.set_sidebar_tree(Rc::new(slint::VecModel::from(sidebar_nodes)).into());
                ui.set_sidebar_loading(true);
            });
        });
    }

    fn handle_test_ok(ww: slint::Weak<crate::AppWindow>) {
        // clone required: invoke_from_event_loop closure must be 'static
        let _ = slint::invoke_from_event_loop(move || {
            with_ui(&ww, |ui| {
                ui.set_form_testing(false);
                ui.set_form_test_ok(true);
                ui.set_test_result_ok(true);
                ui.set_test_result_message("".into());
                ui.set_show_test_result_popup(true);
            });
        });
    }

    fn handle_test_failed(msg: String, ww: slint::Weak<crate::AppWindow>) {
        // clone required: invoke_from_event_loop closure must be 'static
        let _ = slint::invoke_from_event_loop(move || {
            with_ui(&ww, move |ui| {
                ui.set_form_testing(false);
                ui.set_form_test_ok(false);
                ui.set_test_result_ok(false);
                ui.set_test_result_message(msg.into());
                ui.set_show_test_result_popup(true);
            });
        });
    }

    fn handle_connect_error(msg: String, ww: slint::Weak<crate::AppWindow>) {
        // clone required: invoke_from_event_loop closure must be 'static
        let _ = slint::invoke_from_event_loop(move || {
            with_ui(&ww, |ui| {
                ui.set_form_testing(false);
                ui.set_form_status(msg.clone().into());
                ui.set_status_message(format!("Connection failed: {msg}").into());
                ui.set_sidebar_loading(false);
            });
        });
    }

    fn handle_query_started(ww: slint::Weak<crate::AppWindow>) {
        // clone required: invoke_from_event_loop closure must be 'static
        let _ = slint::invoke_from_event_loop(move || {
            with_ui(&ww, |ui| {
                ui.set_is_loading(true);
                ui.set_error_message("".into());
                ui.set_status_message("Running\u{2026}".into());
                ui.set_result_panel_open(true);
            });
        });
    }

    fn handle_query_finished(
        result: QueryResult,
        ww: slint::Weak<crate::AppWindow>,
        original_data: SharedOriginalData,
    ) {
        // Build Send data outside invoke_from_event_loop (Rc<VecModel> is not Send).
        let col_count = result.columns.len();
        let columns: Vec<slint::SharedString> =
            result.columns.iter().map(|c| c.clone().into()).collect();
        let raw_rows: Vec<Vec<Option<String>>> = result.rows.iter().map(|r| r.to_vec()).collect();
        let row_count = result.row_count as i32;
        let exec_ms = result.execution_time_ms;
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
        let _ = slint::invoke_from_event_loop(move || {
            with_ui(&ww, move |ui| {
                ui.set_is_loading(false);
                ui.set_result_active_filter("".into());
                ui.set_result_sort_col(-1);
                ui.set_result_sort_asc(true);
                let col_model = Rc::new(slint::VecModel::from(columns));
                ui.set_result_columns(col_model.into());
                let rows: Vec<crate::RowData> = raw_rows.into_iter().map(rows_to_ui).collect();
                ui.set_result_rows(Rc::new(slint::VecModel::from(rows)).into());
                ui.set_result_row_count(row_count);
                ui.set_result_total_rows(row_count);
                let widths: Vec<f32> = vec![DEFAULT_COLUMN_WIDTH; col_count];
                let total_w = col_count as f32 * DEFAULT_COLUMN_WIDTH;
                ui.set_result_col_widths(Rc::new(slint::VecModel::from(widths)).into());
                ui.set_result_total_col_width(total_w);
                ui.set_status_message(format!("{exec_ms} ms  ·  {row_count} rows").into());
                ui.set_result_panel_open(true);
            });
        });
    }

    fn handle_query_cancelled(ww: slint::Weak<crate::AppWindow>) {
        // clone required: invoke_from_event_loop closure must be 'static
        let _ = slint::invoke_from_event_loop(move || {
            with_ui(&ww, |ui| {
                ui.set_is_loading(false);
                ui.set_status_message("Cancelled".into());
            });
        });
    }

    fn handle_query_error(msg: String, ww: slint::Weak<crate::AppWindow>) {
        let summary = msg
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or(&msg)
            .chars()
            .take(ERROR_TRUNCATION_CHARS)
            .collect::<String>();
        // clone required: invoke_from_event_loop closure must be 'static
        let _ = slint::invoke_from_event_loop(move || {
            with_ui(&ww, move |ui| {
                ui.set_is_loading(false);
                ui.set_form_status(msg.clone().into());
                ui.set_form_testing(false);
                ui.set_error_message(msg.into());
                ui.set_status_message(format!("Error: {summary}").into());
                ui.set_result_panel_open(true);
            });
        });
    }

    fn handle_disconnected(id: String, ww: slint::Weak<crate::AppWindow>) {
        // clone required: invoke_from_event_loop closure must be 'static
        let _ = slint::invoke_from_event_loop(move || {
            with_ui(&ww, move |ui| {
                ui.set_status_message(format!("Disconnected: {id}").into());
                ui.set_status_connection("Not connected".into());
            });
        });
    }

    fn handle_metadata_loaded(
        conn_id: String,
        meta: DbMetadata,
        ww: slint::Weak<crate::AppWindow>,
        state: SharedState,
        sidebar_state: Arc<Mutex<SidebarUiState>>,
    ) {
        {
            let mut sb = sidebar_state.lock().unwrap_or_else(|p| p.into_inner());
            sb.metadata.insert(conn_id, meta);
        }
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
        let _ = slint::invoke_from_event_loop(move || {
            with_ui(&ww, move |ui| {
                ui.set_sidebar_tree(Rc::new(slint::VecModel::from(nodes)).into());
                ui.set_sidebar_loading(false);
            });
        });
    }

    fn handle_metadata_fetch_failed(msg: String, ww: slint::Weak<crate::AppWindow>) {
        // clone required: invoke_from_event_loop closure must be 'static
        let _ = slint::invoke_from_event_loop(move || {
            with_ui(&ww, move |ui| {
                ui.set_sidebar_loading(false);
                ui.set_status_message(format!("Metadata unavailable: {msg}").into());
            });
        });
    }

    fn handle_insert_text(text: String, ww: slint::Weak<crate::AppWindow>) {
        // clone required: invoke_from_event_loop closure must be 'static
        let _ = slint::invoke_from_event_loop(move || {
            with_ui(&ww, move |ui| {
                let current = ui.get_editor_text().to_string();
                ui.set_editor_text(append_editor_text(&current, &text).into());
            });
        });
    }

    fn handle_completion_ready(items: Vec<CompletionItem>, ww: slint::Weak<crate::AppWindow>) {
        // Build Vec<CompletionRow> outside invoke_from_event_loop (Vec is Send, Rc is not).
        let rows: Vec<crate::CompletionRow> = items
            .iter()
            .map(|item| crate::CompletionRow {
                label: item.label.clone().into(),
                kind: completion_kind_label(&item.kind).into(),
                detail: item.detail.clone().unwrap_or_default().into(),
                insert_text: item.insert_text.clone().into(),
                cursor_offset: item.cursor_offset,
                table_name: item.table_name.clone().unwrap_or_default().into(),
            })
            .collect();
        // clone required: invoke_from_event_loop closure must be 'static
        let _ = slint::invoke_from_event_loop(move || {
            with_ui(&ww, move |ui| {
                if rows.is_empty() {
                    ui.set_completion_visible(false);
                } else {
                    let model = Rc::new(slint::VecModel::from(rows));
                    ui.set_completion_items(model.into());
                    ui.set_completion_selected(0);
                    ui.set_completion_visible(true);
                }
            });
        });
    }

    fn handle_theme_changed(t: Theme, ww: slint::Weak<crate::AppWindow>) {
        let is_dark = t == Theme::Dark;
        // clone required: invoke_from_event_loop closure must be 'static
        let _ = slint::invoke_from_event_loop(move || {
            with_ui(&ww, |ui| ui.set_is_dark(is_dark));
        });
    }

    // ── Theme callback ────────────────────────────────────────────────────────

    fn register_theme_callback(
        window: &crate::AppWindow,
        state: SharedState,
        tx_cmd: mpsc::Sender<Command>,
    ) {
        let ui = window.global::<crate::UiState>();
        let window_weak = window.as_weak(); // clone required: on_toggle_theme closure
        ui.on_toggle_theme(move || {
            // Optimistic update: flip is-dark immediately on the UI thread.
            with_ui(&window_weak, |ui| {
                let was_dark = ui.get_is_dark();
                ui.set_is_dark(!was_dark);
                let new_theme = if was_dark { Theme::Light } else { Theme::Dark };
                state.ui.set_theme(new_theme.clone());
                send_cmd(
                    &tx_cmd,
                    Command::UpdateConfig(ConfigUpdate::Theme(new_theme)),
                );
            });
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
                with_ui(&window_weak, |ui| {
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
            });
        }

        // toggle-sidebar-node: expand/collapse a tree node; also switches active
        // connection when an inactive level-0 (connection) node is clicked.
        {
            // clone required: callback closure needs owned captures
            let tx_cmd = tx_cmd.clone();
            let state = state.clone();
            let sidebar_state = Arc::clone(&sidebar_state);
            let window_weak = window.as_weak();
            ui_state.on_toggle_sidebar_node(move |id| {
                let id = id.to_string();
                // For connection nodes, switch only when not already active.
                if let Some(conn_id) = id.strip_prefix("conn:") {
                    let active_id = state
                        .conn
                        .active()
                        .map(|c| c.id.clone())
                        .unwrap_or_default();
                    if conn_id != active_id {
                        let conn = state.conn.all().into_iter().find(|c| c.id == conn_id);
                        if let Some(conn) = conn {
                            let password = conn
                                .password_encrypted
                                .as_ref()
                                .and_then(|enc| crypto::decrypt(enc, &enc_key).ok());
                            send_cmd(&tx_cmd, Command::Connect(conn, password));
                        }
                        // Return early — Event::Connected will auto-expand the newly active node.
                        return;
                    }
                }
                // Toggle expanded state (active connection and category nodes).
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
                with_ui(&window_weak, |ui| {
                    ui.set_sidebar_tree(Rc::new(slint::VecModel::from(nodes)).into());
                });
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
                with_ui(&window_weak, |ui| {
                    let current = ui.get_editor_text().to_string();
                    ui.set_editor_text(append_editor_text(&current, &sql).into());
                });
                send_cmd(&tx_cmd, Command::RunQuery(sql));
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
                with_ui(&window_weak, |ui| ui.set_show_connection_form(false));
            });
        }

        // test-connection: probe without saving — sends Command::TestConnection
        {
            let window_weak = window.as_weak();
            // clone required: callback closure needs owned tx_cmd
            let tx_cmd = tx_cmd.clone();
            ui_state.on_test_connection(move || {
                with_ui(&window_weak, |ui| {
                    ui.set_form_testing(true);
                    ui.set_form_status("".into());
                    ui.set_form_test_ok(false);
                    let (conn, password) = build_conn_from_form(ui, &enc_key);
                    send_cmd(&tx_cmd, Command::TestConnection(conn, password));
                });
            });
        }

        // add-connection: persist if test passed, else show confirm popup
        {
            let window_weak = window.as_weak();
            // clone required: callback closure needs owned tx_cmd
            let tx_cmd = tx_cmd.clone();
            ui_state.on_add_connection(move || {
                with_ui(&window_weak, |ui| {
                    if ui.get_form_test_ok() {
                        ui.set_form_testing(true);
                        let (conn, password) = build_conn_from_form(ui, &enc_key);
                        send_cmd(&tx_cmd, Command::Connect(conn, password));
                    } else {
                        ui.set_show_add_confirm_popup(true);
                    }
                });
            });
        }

        // confirm-add-connection: user chose "Yes" in confirm popup
        {
            let window_weak = window.as_weak();
            // clone required: callback closure needs owned tx_cmd
            let tx_cmd = tx_cmd.clone();
            ui_state.on_confirm_add_connection(move || {
                with_ui(&window_weak, |ui| {
                    ui.set_show_add_confirm_popup(false);
                    ui.set_form_testing(true);
                    let (conn, password) = build_conn_from_form(ui, &enc_key);
                    send_cmd(&tx_cmd, Command::Connect(conn, password));
                });
            });
        }

        // dismiss-test-popup: close the test-result popup
        {
            let window_weak = window.as_weak();
            ui_state.on_dismiss_test_popup(move || {
                with_ui(&window_weak, |ui| ui.set_show_test_result_popup(false));
            });
        }

        // dismiss-add-confirm: user chose "No" in confirm popup
        {
            let window_weak = window.as_weak();
            ui_state.on_dismiss_add_confirm(move || {
                with_ui(&window_weak, |ui| ui.set_show_add_confirm_popup(false));
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
                send_cmd(&tx_cmd, Command::RunQuery(sql.to_string()));
            });
        }
        {
            let tx_cmd = tx_cmd.clone(); // clone required: callback closure needs owned tx_cmd
            ui.on_cancel_query(move || {
                send_cmd(&tx_cmd, Command::CancelQuery);
            });
        }
    }

    // ── Completion callbacks ──────────────────────────────────────────────────

    fn register_completion_callbacks(window: &crate::AppWindow, tx_cmd: mpsc::Sender<Command>) {
        use std::cell::RefCell;
        use std::rc::Rc;
        use std::time::Duration;

        let ui = window.global::<crate::UiState>();

        // Debounced path (text-change → 300 ms → FetchCompletion).
        // Dropping the previous timer stops it — each keystroke resets the window.
        let debounce: Rc<RefCell<Option<slint::Timer>>> = Rc::new(RefCell::new(None));
        {
            let debounce = debounce.clone(); // clone required: on_fetch_completion closure
            let tx_cmd = tx_cmd.clone(); // clone required: on_fetch_completion closure
            ui.on_fetch_completion(move |sql, cursor_pos| {
                *debounce.borrow_mut() = None; // drop previous timer → cancels it
                let tx = tx_cmd.clone(); // clone required: Timer callback
                let sql = sql.to_string();
                let timer = slint::Timer::default();
                timer.start(
                    slint::TimerMode::SingleShot,
                    Duration::from_millis(COMPLETION_DEBOUNCE_MS),
                    move || {
                        let sql = sql.clone();
                        send_cmd(&tx, Command::FetchCompletion(sql, cursor_pos as usize));
                    },
                );
                *debounce.borrow_mut() = Some(timer);
            });
        }

        // Immediate path (Ctrl+Space → FetchCompletion without delay).
        {
            ui.on_trigger_completion(move |sql, cursor_pos| {
                send_cmd(
                    &tx_cmd,
                    Command::FetchCompletion(sql.to_string(), cursor_pos as usize),
                );
            });
        }
    }

    fn register_completion_accept_callback(window: &crate::AppWindow) {
        let ui = window.global::<crate::UiState>();
        let window_weak = window.as_weak(); // clone required: on_accept_completion closure
        ui.on_accept_completion(
            move |insert_text, cursor_pos, cursor_offset_val, table_name| {
                with_ui(&window_weak, move |ui| {
                    let current = ui.get_editor_text().to_string();
                    let pos = (cursor_pos as usize).min(current.len());
                    let mut prefix_start = find_prefix_start(&current, pos);
                    // When accepting a disambiguated column candidate (table_name is set), the
                    // user may have typed "colname tableprefix" with a space.  Extend the
                    // replacement range backward to cover the entire "colname tableprefix" so the
                    // insertion replaces both words, not just the current word after the space.
                    let table_name_str = table_name.to_string();
                    if !table_name_str.is_empty() {
                        let before_prefix = &current[..prefix_start];
                        let pattern = format!("{} ", insert_text.as_str());
                        if before_prefix.ends_with(&pattern) {
                            let extended = prefix_start - pattern.len();
                            let at_boundary = extended == 0
                                || matches!(
                                    current.as_bytes().get(extended - 1),
                                    Some(b' ') | Some(b'\t') | Some(b'\n')
                                );
                            if at_boundary {
                                prefix_start = extended;
                            }
                        }
                    }

                    // If the accepted text is a SQL keyword (FROM, WHERE, AND …), treat it as
                    // a plain insertion even inside a SELECT list — no comma should be added.
                    let is_keyword = wf_completion::parser::is_sql_keyword(insert_text.as_str());
                    let in_select =
                        !is_keyword && wf_completion::parser::in_select_list(&current, pos);

                    let (new_text, new_cursor): (String, i32) = if in_select {
                        // In SELECT list: auto-insert ", " between columns.
                        let trimmed = current[..prefix_start].trim_end_matches([' ', '\t']);
                        let last_char = trimmed.chars().last();
                        let last_word_start = trimmed
                            .rfind(|c: char| !c.is_alphanumeric() && c != '_')
                            .map(|i| i + 1)
                            .unwrap_or(0);
                        let last_word = trimmed[last_word_start..].to_ascii_uppercase();
                        let needs_comma = !matches!(last_char, None | Some(',') | Some('('))
                            && !matches!(last_word.as_str(), "SELECT" | "DISTINCT");
                        if needs_comma {
                            let text = format!("{}, {}{}", trimmed, insert_text, &current[pos..]);
                            let cur = (trimmed.len() + 2 + insert_text.len()) as i32;
                            (text, cur)
                        } else {
                            let text = format!(
                                "{}{}{}",
                                &current[..prefix_start],
                                insert_text,
                                &current[pos..]
                            );
                            let cur = (prefix_start + insert_text.len()) as i32;
                            (text, cur)
                        }
                    } else {
                        // Determine whether to replace the typed prefix or insert at cursor.
                        // When the accepted text is unrelated to the prefix (e.g. user finished
                        // typing "users" and now accepts a NextClause keyword like "WHERE"),
                        // insert at the cursor position with a leading space rather than
                        // overwriting the table/column name.
                        let prefix_word = &current[prefix_start..pos];
                        let (actual_start, add_leading_space) = if prefix_word.is_empty() {
                            // Cursor is at whitespace or string start — plain insert.
                            (pos, false)
                        } else if insert_text
                            .as_str()
                            .to_ascii_uppercase()
                            .starts_with(&prefix_word.to_ascii_uppercase())
                        {
                            // Prefix is a partial match of insert_text — replace it.
                            (prefix_start, false)
                        } else {
                            // Prefix is unrelated (e.g. "users" + "WHERE") — insert at cursor.
                            let needs_space =
                                !current[..pos].ends_with(|c: char| c.is_ascii_whitespace());
                            (pos, needs_space)
                        };
                        let leading = if add_leading_space { " " } else { "" };
                        let text = format!(
                            "{}{}{}{}",
                            &current[..actual_start],
                            leading,
                            insert_text,
                            &current[pos..]
                        );
                        let cur = if cursor_offset_val > 0 {
                            actual_start as i32 + add_leading_space as i32 + cursor_offset_val
                        } else {
                            (actual_start + leading.len() + insert_text.len()) as i32
                        };
                        (text, cur)
                    };

                    // Auto-append FROM <table> when a column with a known table was accepted
                    // inside a SELECT list that has no FROM clause yet.
                    let appended_from =
                        in_select && !table_name_str.is_empty() && !sql_has_from(&current);
                    let (final_text, final_cursor) = if appended_from {
                        let appended = format!("{} FROM {}", new_text.trim_end(), table_name_str);
                        let cur = appended.len() as i32;
                        (appended, cur)
                    } else {
                        (new_text, new_cursor)
                    };

                    // inserted text, e.g. between the quotes in `''`).
                    let at_end = final_cursor as usize == final_text.len();
                    ui.set_editor_text(final_text.clone().into());
                    ui.set_editor_cursor_target(final_cursor);
                    // Re-trigger only when the cursor is at end of inserted text AND the text
                    // does not end at a syntactically terminal expression (IS NULL, TRUE, FALSE,
                    // a string/numeric literal, ASC/DESC).  Terminal positions use a virtual
                    // trailing space so the parser sees the next context without polluting the
                    // editor with a stale space the user would have to delete before typing `;`.
                    if cursor_offset_val == 0 && at_end && !is_terminal_expression(&final_text) {
                        let trigger_sql = format!("{} ", final_text);
                        ui.invoke_trigger_completion(trigger_sql.into(), final_cursor + 1);
                    }
                });
            },
        );
    }

    fn register_formatter_callback(window: &crate::AppWindow) {
        let ui = window.global::<crate::UiState>();
        let window_weak = window.as_weak(); // clone required: on_format_sql closure
        ui.on_format_sql(move || {
            with_ui(&window_weak, |ui| {
                let text = ui.get_editor_text().to_string();
                let formatted = wf_query::formatter::format_sql(&text);
                ui.set_editor_text(formatted.into());
            });
        });
    }

    const CSV_DEFAULT_FILENAME: &str = "query_result.csv";
    const JSON_DEFAULT_FILENAME: &str = "query_result.json";

    fn register_export_callbacks(window: &crate::AppWindow, original_data: SharedOriginalData) {
        let ui = window.global::<crate::UiState>();

        // ── CSV export ────────────────────────────────────────────────────────
        let window_weak = window.as_weak(); // clone required: on_export_csv closure
        {
            let original_data = Arc::clone(&original_data); // clone required: on_export_csv closure
            ui.on_export_csv(move || {
                // Snapshot columns + rows while still on the UI thread (Mutex is not Send).
                let snapshot = {
                    let orig = original_data.lock().unwrap_or_else(|p| p.into_inner());
                    orig.as_ref().map(|d| {
                        let cols: Vec<String> = d.columns.iter().map(|s| s.to_string()).collect();
                        (cols, d.rows.clone())
                    })
                };
                let Some((columns, rows)) = snapshot else {
                    return;
                };
                let window_weak = window_weak.clone(); // clone required: tokio::spawn needs 'static
                tokio::spawn(async move {
                    let Some(handle) = rfd::AsyncFileDialog::new()
                        .set_title("Save CSV")
                        .set_file_name(Self::CSV_DEFAULT_FILENAME)
                        .add_filter("CSV files", &["csv"])
                        .save_file()
                        .await
                    else {
                        return; // user cancelled
                    };
                    let path = handle.path().to_path_buf();
                    let result = wf_query::export::export_csv(&columns, &rows, &path);
                    let msg = match result {
                        Ok(()) => format!("Saved CSV: {}", path.display()),
                        Err(e) => format!("CSV export failed: {e}"),
                    };
                    set_status(window_weak, msg);
                });
            });
        }

        // ── JSON export ───────────────────────────────────────────────────────
        {
            let window_weak = window.as_weak(); // clone required: on_export_json closure
            let original_data = Arc::clone(&original_data); // clone required: on_export_json closure
            ui.on_export_json(move || {
                let snapshot = {
                    let orig = original_data.lock().unwrap_or_else(|p| p.into_inner());
                    orig.as_ref().map(|d| {
                        let cols: Vec<String> = d.columns.iter().map(|s| s.to_string()).collect();
                        (cols, d.rows.clone())
                    })
                };
                let Some((columns, rows)) = snapshot else {
                    return;
                };
                let window_weak = window_weak.clone(); // clone required: tokio::spawn needs 'static
                tokio::spawn(async move {
                    let Some(handle) = rfd::AsyncFileDialog::new()
                        .set_title("Save JSON")
                        .set_file_name(Self::JSON_DEFAULT_FILENAME)
                        .add_filter("JSON files", &["json"])
                        .save_file()
                        .await
                    else {
                        return; // user cancelled
                    };
                    let path = handle.path().to_path_buf();
                    let result = wf_query::export::export_json(&columns, &rows, &path);
                    let msg = match result {
                        Ok(()) => format!("Saved JSON: {}", path.display()),
                        Err(e) => format!("JSON export failed: {e}"),
                    };
                    set_status(window_weak, msg);
                });
            });
        }
    }

    // ── Result callbacks ──────────────────────────────────────────────────────

    fn register_result_callbacks(
        window: &crate::AppWindow,
        state: SharedState,
        original_data: SharedOriginalData,
        tx_cmd: mpsc::Sender<Command>,
    ) {
        let ui_state = window.global::<crate::UiState>();
        let window_weak = window.as_weak();

        // resize-result-column: update the column width VecModel in place and
        // recompute the total so viewport-width stays accurate during drag.
        {
            // clone required: callback closure must be 'static
            let window_weak = window_weak.clone();
            ui_state.on_resize_result_column(move |i, w| {
                with_ui(&window_weak, |ui| {
                    let model = ui.get_result_col_widths();
                    let n = model.row_count();
                    if (i as usize) < n {
                        model.set_row_data(i as usize, w);
                        let total: f32 = (0..n).filter_map(|j| model.row_data(j)).sum();
                        ui.set_result_total_col_width(total);
                    }
                });
            });
        }

        // filter-result-rows: apply client-side predicate, then re-apply active sort.
        {
            // clone required: callback closure must be 'static
            let window_weak = window_weak.clone();
            let original_data = Arc::clone(&original_data);
            ui_state.on_filter_result_rows(move |query| {
                with_ui(&window_weak, |ui| {
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
            });
        }

        // clear-result-filter: restore the unfiltered original rows, then re-apply active sort.
        {
            // clone required: callback closure must be 'static
            let window_weak = window_weak.clone();
            let original_data = Arc::clone(&original_data);
            ui_state.on_clear_result_filter(move || {
                with_ui(&window_weak, |ui| {
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

        // copy-result-row: join visible row i cells with tabs, NULL → empty string.
        {
            // clone required: callback closure must be 'static
            let window_weak = window_weak.clone();
            ui_state.on_copy_result_row(move |row_i| {
                with_ui(&window_weak, |ui| {
                    let rows_model = ui.get_result_rows();
                    if let Some(row) = rows_model.row_data(row_i as usize) {
                        let cells: Vec<Option<String>> = (0..row.cells.row_count())
                            .filter_map(|j| row.cells.row_data(j))
                            .map(|c| {
                                if c.is_null {
                                    None
                                } else {
                                    Some(c.value.to_string())
                                }
                            })
                            .collect();
                        let tsv = cells_to_tsv(&cells);
                        if let Ok(mut clip) = arboard::Clipboard::new() {
                            let _ = clip.set_text(tsv);
                        }
                    }
                });
            });
        }

        // copy-result-tsv: export all visible rows as TSV with column headers.
        {
            // clone required: callback closure must be 'static
            let window_weak = window_weak.clone();
            ui_state.on_copy_result_tsv(move || {
                with_ui(&window_weak, |ui| {
                    let cols_model = ui.get_result_columns();
                    let rows_model = ui.get_result_rows();
                    let columns: Vec<String> = (0..cols_model.row_count())
                        .filter_map(|i| cols_model.row_data(i))
                        .map(|s| s.to_string())
                        .collect();
                    let rows: Vec<Vec<Option<String>>> = (0..rows_model.row_count())
                        .filter_map(|i| rows_model.row_data(i))
                        .map(|row| {
                            (0..row.cells.row_count())
                                .filter_map(|j| row.cells.row_data(j))
                                .map(|c| {
                                    if c.is_null {
                                        None
                                    } else {
                                        Some(c.value.to_string())
                                    }
                                })
                                .collect()
                        })
                        .collect();
                    let col_strs: Vec<&str> = columns.iter().map(String::as_str).collect();
                    let tsv = result_to_tsv(&col_strs, &rows);
                    if let Ok(mut clip) = arboard::Clipboard::new() {
                        let _ = clip.set_text(tsv);
                    }
                });
            });
        }

        // update-page-size: user clicked 100/500/1000 in the result toolbar
        // (ALL / 0 goes through confirm-all-rows instead).
        // 1. Update UiState.page-size immediately so the button highlight changes.
        // 2. Update shared state so the injected LIMIT is correct for the rerun.
        // 3. Persist via UpdateConfig (0 has no PageSize variant yet — skipped).
        // 4. Auto-rerun the last query with the new limit.
        {
            // clone required: callback closure must be 'static
            let window_weak = window_weak.clone();
            let tx_cmd = tx_cmd.clone();
            let state_rerun = state.clone(); // clone required: captured by callback
            ui_state.on_update_page_size(move |n| {
                let size = n as usize;
                state_rerun.ui.set_page_size(size);
                with_ui(&window_weak, |ui| ui.set_page_size(n));
                if let Ok(ps) = wf_config::models::PageSize::try_from(n as u32) {
                    send_cmd(&tx_cmd, Command::UpdateConfig(ConfigUpdate::PageSize(ps)));
                }
                // Auto-rerun the last query so results reflect the new limit immediately.
                if let Some(last_sql) = state_rerun.query.last_sql() {
                    send_cmd(&tx_cmd, Command::RunQuery(last_sql));
                }
            });
        }

        // confirm-all-rows: user confirmed the "fetch all rows" popup.
        // Sets page-size=0 (no LIMIT), closes the popup, then reruns the last query.
        {
            // clone required: callback closure must be 'static
            let window_weak = window_weak.clone();
            let tx_cmd = tx_cmd.clone();
            let state_all = state.clone(); // clone required: captured by callback
            ui_state.on_confirm_all_rows(move || {
                state_all.ui.set_page_size(0);
                with_ui(&window_weak, |ui| {
                    ui.set_page_size(0);
                    ui.set_show_all_rows_confirm(false);
                });
                if let Some(last_sql) = state_all.query.last_sql() {
                    send_cmd(&tx_cmd, Command::RunQuery(last_sql));
                }
            });
        }

        // dismiss-all-rows-confirm: user cancelled the "fetch all rows" popup.
        {
            // clone required: callback closure must be 'static
            let window_weak = window_weak.clone();
            ui_state.on_dismiss_all_rows_confirm(move || {
                with_ui(&window_weak, |ui| ui.set_show_all_rows_confirm(false));
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
                with_ui(&window_weak, |ui| {
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

fn completion_kind_label(kind: &wf_completion::CompletionKind) -> &'static str {
    use wf_completion::CompletionKind::*;
    match kind {
        Table => "Table",
        Column => "Column",
        Keyword => "Keyword",
        Operator => "Operator",
        Schema => "Schema",
        View => "View",
    }
}

/// Returns the byte offset of the first character of the word immediately
/// before `cursor_pos` in `text`.  Handles dot-qualified names (e.g. `u.em`)
/// by only considering the segment after the last `.`.
pub(crate) fn find_prefix_start(text: &str, cursor_pos: usize) -> usize {
    let pos = cursor_pos.min(text.len());
    let before = &text[..pos];
    let search_start = before.rfind('.').map(|i| i + 1).unwrap_or(0);
    let prefix_len = before[search_start..]
        .bytes()
        .rev()
        .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
        .count();
    pos - prefix_len
}

/// Returns `true` when `text` ends at a position where showing the next
/// completion popup would be misleading — specifically after terminal SQL
/// expressions: IS NULL, IS NOT NULL, TRUE, FALSE, ASC, DESC, string
/// literals (ending `'`), or numeric literals.
///
/// This suppresses the auto-retrigger after, e.g., `IS NOT NULL` so the
/// editor does not immediately show column candidates for the next `AND`
/// clause; the user can still invoke Ctrl+Space explicitly if needed.
fn is_terminal_expression(text: &str) -> bool {
    let t = text.trim_end();
    // Last whole-word token (alphanumeric + underscore run after final separator).
    let last_word_start = t
        .rfind(|c: char| !c.is_alphanumeric() && c != '_')
        .map(|i| i + 1)
        .unwrap_or(0);
    let last_word = t[last_word_start..].to_ascii_uppercase();
    if matches!(
        last_word.as_str(),
        "NULL" | "TRUE" | "FALSE" | "ASC" | "DESC"
    ) {
        return true;
    }
    // String literal or numeric literal.
    t.ends_with('\'') || t.ends_with('"') || t.chars().last().is_some_and(|c| c.is_ascii_digit())
}

/// Returns `true` if `sql` contains a FROM clause (case-insensitive).
/// Used to decide whether to auto-append `FROM <table>` after a column acceptance.
fn sql_has_from(sql: &str) -> bool {
    let upper = sql.to_ascii_uppercase();
    upper.contains(" FROM ")
        || upper.contains("\nFROM ")
        || upper.contains("\tFROM ")
        || upper.starts_with("FROM ")
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

/// Join one row's cells as a TSV line. `None` (NULL) → empty string.
fn cells_to_tsv(cells: &[Option<String>]) -> String {
    cells
        .iter()
        .map(|c| c.as_deref().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\t")
}

/// Format `columns` + `rows` as a TSV string with a header line.
fn result_to_tsv(columns: &[&str], rows: &[Vec<Option<String>>]) -> String {
    let mut lines = Vec::with_capacity(rows.len() + 1);
    lines.push(columns.join("\t"));
    for row in rows {
        lines.push(cells_to_tsv(row));
    }
    lines.join("\n")
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

#[cfg(test)]
mod tests;

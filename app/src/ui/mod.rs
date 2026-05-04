#![allow(dead_code)]

mod tabs_state;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use rust_i18n::t;
use slint::ComponentHandle;
use slint::Model as _;
use tokio::sync::mpsc;
use wf_completion::CompletionItem;
use wf_config::{crypto, snippet::SnippetRepository};
use wf_db::models::{DbConnection, DbMetadata, DbType, QueryResult, TableInfo};
use wf_history::find_history::FindHistoryService;
use wf_history::session::SessionService;
use wf_query::analyzer::{extract_statement_at, has_dangerous_dml};

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

/// Returns true if a safe-DML warning was shown (caller should return early).
/// Reads conn-safe-dml from UiState; if enabled and SQL is dangerous, shows dialog.
fn check_safe_dml(weak: &slint::Weak<crate::AppWindow>, sql: &str, kind: &str) -> bool {
    let Some(w) = weak.upgrade() else {
        return false;
    };
    let ui = w.global::<crate::UiState>();
    if !ui.get_conn_safe_dml() {
        return false;
    }
    if !has_dangerous_dml(sql) {
        return false;
    }
    ui.set_safe_dml_pending_sql(sql.into());
    ui.set_safe_dml_pending_kind(kind.into());
    ui.set_show_safe_dml_confirm(true);
    true
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

// ── Find / replace helpers ────────────────────────────────────────────────────

/// History data shared between the tokio task that loads from SQLite and the
/// UI-thread callbacks that navigate it.  Arc<Mutex<>> because it crosses the
/// async boundary; actual navigation index lives in FindState (UI thread only).
#[derive(Default)]
struct HistorySnapshot {
    find: Vec<String>, // newest-first (index 0 = most recently inserted)
    replace: Vec<String>,
}

type SharedHistorySnapshot = Arc<std::sync::Mutex<HistorySnapshot>>;

/// Cached state for the find bar — avoids re-scanning on next/prev when the
/// query and text have not changed.
#[derive(Default)]
struct FindState {
    last_text: String,
    last_query: String,
    last_case_sensitive: bool,
    last_use_regex: bool,
    matches: Vec<(usize, usize)>, // (start_byte, end_byte)
    current: usize,
    // History navigation (UI thread only)
    find_hist_idx: Option<usize>, // None = not browsing; Some(i) = position in snapshot.find
    replace_hist_idx: Option<usize>,
    find_draft: String, // query captured before history browsing started
    replace_draft: String,
}

impl FindState {
    /// Re-compute matches when any parameter changed; clamps `current`.
    fn update(&mut self, text: &str, query: &str, case_sensitive: bool, use_regex: bool) {
        if text == self.last_text
            && query == self.last_query
            && case_sensitive == self.last_case_sensitive
            && use_regex == self.last_use_regex
        {
            return;
        }
        self.matches = compute_matches(text, query, case_sensitive, use_regex);
        self.last_text = text.to_string();
        self.last_query = query.to_string();
        self.last_case_sensitive = case_sensitive;
        self.last_use_regex = use_regex;
        if self.matches.is_empty() {
            self.current = 0;
        } else {
            self.current = self.current.min(self.matches.len() - 1);
        }
    }

    fn params_changed(&self, query: &str, case_sensitive: bool, use_regex: bool) -> bool {
        query != self.last_query
            || case_sensitive != self.last_case_sensitive
            || use_regex != self.last_use_regex
    }
}

/// Returns all (start_byte, end_byte) positions where `query` matches in `text`.
pub(crate) fn compute_matches(
    text: &str,
    query: &str,
    case_sensitive: bool,
    use_regex: bool,
) -> Vec<(usize, usize)> {
    if query.is_empty() {
        return vec![];
    }
    let pattern = if use_regex {
        if case_sensitive {
            query.to_string()
        } else {
            format!("(?i){query}")
        }
    } else {
        let escaped = regex::escape(query);
        if case_sensitive {
            escaped
        } else {
            format!("(?i){escaped}")
        }
    };
    match regex::Regex::new(&pattern) {
        Ok(re) => re.find_iter(text).map(|m| (m.start(), m.end())).collect(),
        Err(_) => vec![],
    }
}

use wf_config::models::{ConnectionConfig, DbTypeName, Theme};

use crate::{
    app::{
        command::{Command, ConfigUpdate},
        event::{Event, StateEvent},
        session::config_to_db_conn,
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
    read_only: HashMap<String, bool>,
    /// All saved connections (from ConnectionRepository). Used to build the
    /// sidebar and DB manager list even when no DB is running.
    config_connections: Vec<ConnectionConfig>,
}

fn build_sidebar_tree(
    config_conns: &[ConnectionConfig],
    active_id: &str,
    metadata: &HashMap<String, DbMetadata>,
    expanded: &HashSet<String>,
    read_only: &HashMap<String, bool>,
) -> Vec<crate::SidebarNode> {
    let mut nodes = vec![];
    for conn in config_conns {
        let conn_node_id = format!("conn:{}", conn.id);
        let is_conn_expanded = expanded.contains(&conn_node_id);
        let conn_idx = nodes.len() as i32;
        nodes.push(crate::SidebarNode {
            id: conn_node_id.clone().into(),
            label: conn.name.clone().into(),
            sub_label: db_type_label_config(&conn.db_type).into(),
            level: 0,
            is_expanded: is_conn_expanded,
            is_active: conn.id == active_id,
            is_read_only: *read_only.get(&conn.id).unwrap_or(&false),
            node_kind: "connection".into(),
            parent_index: -1,
            visible: true,
            stagger_delay: 0,
        });
        // Always push all children (with visible flag) so Slint can animate height/opacity.
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
            is_conn_expanded,
        );
        push_tableinfo_category(
            &mut nodes,
            conn_idx,
            &conn.id,
            "Views",
            &meta.views,
            "view",
            expanded,
            is_conn_expanded,
        );
        push_string_category(
            &mut nodes,
            conn_idx,
            &conn.id,
            "Stored Procedures",
            &meta.stored_procs,
            "proc",
            expanded,
            is_conn_expanded,
        );
        push_string_category(
            &mut nodes,
            conn_idx,
            &conn.id,
            "Indexes",
            &meta.indexes,
            "index",
            expanded,
            is_conn_expanded,
        );
    }
    nodes
}

#[allow(clippy::too_many_arguments)]
fn push_tableinfo_category(
    nodes: &mut Vec<crate::SidebarNode>,
    conn_idx: i32,
    conn_id: &str,
    name: &str,
    items: &[TableInfo],
    kind: &str,
    expanded: &HashSet<String>,
    parent_visible: bool,
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
        is_read_only: false,
        node_kind: "category".into(),
        parent_index: conn_idx,
        visible: parent_visible,
        stagger_delay: 0,
    });
    // Always emit children; visible flag drives Slint height/opacity animation.
    for (child_idx, item) in items.iter().enumerate() {
        nodes.push(crate::SidebarNode {
            id: format!("item:{}:{}:{}", conn_id, kind, item.name).into(),
            label: item.name.clone().into(),
            sub_label: "".into(),
            level: 2,
            is_expanded: false,
            is_active: false,
            is_read_only: false,
            node_kind: kind.into(),
            parent_index: cat_idx,
            visible: parent_visible && is_exp,
            stagger_delay: (child_idx.min(9) as i32) * 30,
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn push_string_category(
    nodes: &mut Vec<crate::SidebarNode>,
    conn_idx: i32,
    conn_id: &str,
    name: &str,
    items: &[String],
    kind: &str,
    expanded: &HashSet<String>,
    parent_visible: bool,
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
        is_read_only: false,
        node_kind: "category".into(),
        parent_index: conn_idx,
        visible: parent_visible,
        stagger_delay: 0,
    });
    // Always emit children; visible flag drives Slint height/opacity animation.
    for (child_idx, item) in items.iter().enumerate() {
        nodes.push(crate::SidebarNode {
            id: format!("item:{}:{}:{}", conn_id, kind, item).into(),
            label: item.clone().into(),
            sub_label: "".into(),
            level: 2,
            is_expanded: false,
            is_active: false,
            is_read_only: false,
            node_kind: kind.into(),
            parent_index: cat_idx,
            visible: parent_visible && is_exp,
            stagger_delay: (child_idx.min(9) as i32) * 30,
        });
    }
}

// ---------------------------------------------------------------------------
// Tab helpers
// ---------------------------------------------------------------------------

fn tabs_to_slint(tabs: &[tabs_state::TabEntry]) -> Vec<crate::TabEntry> {
    tabs.iter()
        .map(|t| crate::TabEntry {
            id: t.id.clone().into(),
            title: t.title.clone().into(),
            kind: match &t.kind {
                tabs_state::TabKind::SqlEditor { .. } => "sql-editor".into(),
                tabs_state::TabKind::TableView { .. } => "table-view".into(),
            },
        })
        .collect()
}

fn columns_to_slint(cols: &[wf_db::models::ColumnInfo]) -> Vec<crate::ColumnData> {
    cols.iter()
        .map(|c| crate::ColumnData {
            name: c.name.clone().into(),
            data_type: c.data_type.clone().into(),
            nullable: c.nullable,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// UI
// ---------------------------------------------------------------------------

pub struct UI {
    window: crate::AppWindow,
}

impl UI {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        state: SharedState,
        tx_cmd: mpsc::Sender<Command>,
        rx_event: mpsc::Receiver<Event>,
        enc_key: [u8; 32],
        initial_connections: Vec<ConnectionConfig>,
        find_history_svc: FindHistoryService,
        session_svc: SessionService,
        snippet_repo: Arc<SnippetRepository>,
    ) -> Result<Self> {
        let window = crate::AppWindow::new()?;

        let sidebar_state: Arc<Mutex<SidebarUiState>> = Arc::new(Mutex::new(SidebarUiState {
            read_only: initial_connections
                .iter()
                .map(|c| (c.id.clone(), c.read_only))
                .collect(),
            config_connections: initial_connections,
            ..Default::default()
        }));

        // Shared storage for the unfiltered query result; written by the event
        // handler on QueryFinished, read by the filter callbacks on the UI thread.
        let original_data: SharedOriginalData = Arc::new(Mutex::new(None));

        // Restore or create tab state. Track whether tabs were loaded from DB
        // so we can fall back to last_query on first launch.
        let handle = tokio::runtime::Handle::current();
        let tabs_from_session;
        let tabs_state: Rc<RefCell<tabs_state::TabsState>> = {
            match handle.block_on(session_svc.restore_tabs()) {
                Ok(Some((active_index, entries))) => {
                    tabs_from_session = true;
                    Rc::new(RefCell::new(tabs_state::TabsState::from_session(
                        active_index,
                        entries,
                    )))
                }
                _ => {
                    tabs_from_session = false;
                    Rc::new(RefCell::new(tabs_state::TabsState::new()))
                }
            }
        };

        Self::register_sidebar_callbacks(
            &window,
            state.clone(),
            tx_cmd.clone(),
            Arc::clone(&sidebar_state),
            enc_key,
            Rc::clone(&tabs_state),
        );
        Self::register_connection_form_callbacks(&window, tx_cmd.clone(), enc_key);
        Self::register_editor_callbacks(&window, state.clone(), tx_cmd.clone());
        Self::register_completion_callbacks(&window, tx_cmd.clone());
        Self::register_completion_accept_callback(&window);
        Self::register_formatter_callback(&window);
        Self::register_export_callbacks(&window, Arc::clone(&original_data), state.clone());
        Self::register_theme_callback(&window, state.clone(), tx_cmd.clone());
        Self::register_reduce_motion_callback(&window, tx_cmd.clone());
        Self::register_menu_callbacks(&window, tx_cmd.clone());
        Self::register_close_handler(&window, Rc::clone(&tabs_state), session_svc.clone());
        Self::register_tab_callbacks(
            &window,
            tx_cmd.clone(),
            Rc::clone(&tabs_state),
            Arc::clone(&sidebar_state),
        );
        // Set initial page size and theme on the Slint window from shared state.
        let ui_global = window.global::<crate::UiState>();
        ui_global.set_page_size(state.ui.page_size() as i32);
        ui_global.set_is_dark(state.ui.theme() == Theme::Dark);
        let config = wf_config::manager::ConfigManager::new()
            .load()
            .unwrap_or_default();
        ui_global.set_font_family(config.appearance.font_family.into());
        ui_global.set_font_size(config.appearance.font_size as i32);
        ui_global.set_reduce_motion(config.appearance.reduce_motion);
        // Apply locale after the Slint component exists — select_bundled_translation
        // requires a live component and is a no-op if called before one is created.
        let lang = &config.ui.language;
        let _ = slint::select_bundled_translation(lang);
        rust_i18n::set_locale(lang);
        ui_global.set_language(lang.clone().into());
        // Route UI text through the platform's native renderer (DirectWrite / Core Text)
        // by naming a system font that Slint resolves without falling back to the bundled
        // fontique renderer, which garbles text at certain sizes.
        #[cfg(target_os = "windows")]
        ui_global.set_ui_font("Segoe UI".into());
        #[cfg(target_os = "macos")]
        ui_global.set_ui_font("Helvetica Neue".into());
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        ui_global.set_ui_font("Liberation Sans, DejaVu Sans".into());
        // Initialise tab bar from the restored (or freshly created) tab state.
        {
            let ts = tabs_state.borrow();
            let slint_tabs = tabs_to_slint(&ts.tabs);
            ui_global.set_tabs(Rc::new(slint::VecModel::from(slint_tabs)).into());
            ui_global.set_active_tab_index(ts.active_index as i32);
            match ts.active_tab().map(|t| t.kind.clone()) {
                Some(tabs_state::TabKind::SqlEditor { query_text }) => {
                    ui_global.set_editor_text(query_text.into());
                    ui_global.set_active_tab_kind_sql(true);
                }
                Some(tabs_state::TabKind::TableView { table_name, .. }) => {
                    ui_global.set_tv_table_name(table_name.into());
                    ui_global.set_active_tab_kind_sql(false);
                }
                None => {}
            }
        }
        // Fall back to last_query on first launch (no session tabs saved yet).
        if !tabs_from_session
            && let Ok(Some(query)) = handle.block_on(session_svc.restore_last_query())
        {
            ui_global.set_editor_text(query.clone().into());
            tabs_state.borrow_mut().save_current_text(&query);
        }
        // Populate the connection list and sidebar from all saved connections at startup
        // so they are visible even when no DB is running.
        {
            let sb = sidebar_state.lock().unwrap_or_else(|p| p.into_inner());
            let entries: Vec<crate::ConnectionEntry> = sb
                .config_connections
                .iter()
                .map(|c| crate::ConnectionEntry {
                    is_active: false,
                    db_type: db_type_label_config(&c.db_type).into(),
                    name: c.name.clone().into(),
                    id: c.id.clone().into(),
                })
                .collect();
            ui_global.set_connection_list(Rc::new(slint::VecModel::from(entries)).into());
            let nodes = build_sidebar_tree(
                &sb.config_connections,
                "",
                &sb.metadata,
                &sb.expanded,
                &sb.read_only,
            );
            ui_global.set_sidebar_tree(Rc::new(slint::VecModel::from(nodes)).into());
        }

        Self::register_result_callbacks(
            &window,
            state.clone(),
            Arc::clone(&original_data),
            tx_cmd.clone(),
        );
        Self::register_language_callback(&window, tx_cmd.clone());
        Self::register_find_replace_callbacks(&window, find_history_svc);
        Self::register_snippet_callbacks(&window, Arc::clone(&snippet_repo));
        Self::register_status_callbacks(&window, state.clone());
        Self::register_metadata_search_callbacks(&window, Arc::clone(&sidebar_state));

        // Load initial snippets (global only — no connection yet).
        {
            let initial_bk = handle.block_on(snippet_repo.list(None)).unwrap_or_default();
            let slint_bk: Vec<crate::SnippetEntry> =
                initial_bk.into_iter().map(snippet_to_slint).collect();
            ui_global.set_snippets(Rc::new(slint::VecModel::from(slint_bk)).into());
        }

        // Load persisted Snippet Bar position.
        {
            let (bx, by) = handle
                .block_on(snippet_repo.get_bar_position())
                .unwrap_or((0.0, 100.0));
            ui_global.set_snippet_bar_x(bx);
            ui_global.set_snippet_bar_y(by);
        }

        Self::spawn_event_handler(
            &window,
            rx_event,
            state,
            Arc::clone(&sidebar_state),
            Arc::clone(&original_data),
            Arc::clone(&snippet_repo),
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
        snippet_repo: Arc<SnippetRepository>,
    ) {
        let window_weak = window.as_weak();
        tokio::spawn(async move {
            while let Some(event) = rx_event.recv().await {
                match event {
                    Event::Connected {
                        id,
                        connections,
                        safe_dml,
                        read_only,
                    } => {
                        let conn_id = id.clone();
                        Self::handle_connected(
                            id,
                            connections,
                            safe_dml,
                            read_only,
                            window_weak.clone(),
                            state.clone(),
                            Arc::clone(&sidebar_state),
                        );
                        // Refresh snippets to include per-connection entries.
                        let bk_repo = Arc::clone(&snippet_repo);
                        let bk_ww = window_weak.clone();
                        tokio::spawn(async move {
                            do_refresh_snippets(&bk_ww, &bk_repo, Some(&conn_id)).await;
                        });
                    }
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
                    Event::Disconnected(id) => {
                        Self::handle_disconnected(id, window_weak.clone());
                        // Drop per-connection snippets; show global only.
                        let bk_repo = Arc::clone(&snippet_repo);
                        let bk_ww = window_weak.clone();
                        tokio::spawn(async move {
                            do_refresh_snippets(&bk_ww, &bk_repo, None).await;
                        });
                    }
                    Event::ConnectionRemoved(id) => Self::handle_connection_removed(
                        id,
                        window_weak.clone(),
                        state.clone(),
                        Arc::clone(&sidebar_state),
                    ),
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
                    Event::DdlLoaded { tab_id, ddl } => {
                        Self::handle_ddl_loaded(tab_id, ddl, window_weak.clone())
                    }
                    Event::DdlFetchFailed { tab_id, msg } => {
                        Self::handle_ddl_fetch_failed(tab_id, msg, window_weak.clone())
                    }
                    Event::TableDataLoaded { tab_id, result } => {
                        Self::handle_table_data_loaded(tab_id, result, window_weak.clone())
                    }
                    Event::TableDataFailed { tab_id, msg } => {
                        Self::handle_table_data_failed(tab_id, msg, window_weak.clone())
                    }
                    Event::ConnectionFlagsUpdated {
                        id,
                        safe_dml,
                        read_only,
                    } => Self::handle_connection_flags_updated(
                        id,
                        safe_dml,
                        read_only,
                        window_weak.clone(),
                        state.clone(),
                        Arc::clone(&sidebar_state),
                    ),
                    _ => {}
                }
            }
        });
    }

    // ── Per-event handlers ─────────────────────────────────────────────────────

    fn handle_connected(
        id: String,
        connections: Vec<ConnectionConfig>,
        safe_dml: bool,
        read_only: bool,
        ww: slint::Weak<crate::AppWindow>,
        state: SharedState,
        sidebar_state: Arc<Mutex<SidebarUiState>>,
    ) {
        // Build Send data outside invoke_from_event_loop (Rc<VecModel> is not Send).
        let entries: Vec<crate::ConnectionEntry> = connections
            .iter()
            .map(|c| crate::ConnectionEntry {
                is_active: c.id == id,
                db_type: db_type_label_config(&c.db_type).into(),
                name: c.name.clone().into(),
                id: c.id.clone().into(),
            })
            .collect();
        let base_status = connections
            .iter()
            .find(|c| c.id == id)
            .map(|c| match c.database.as_deref() {
                Some(db) if !db.is_empty() => format!("{} / {}", c.name, db),
                _ => c.name.clone(),
            })
            .unwrap_or_else(|| id.clone());
        let status_conn = if read_only {
            format!("{} · {}", base_status, t!("status.read_only"))
        } else {
            base_status
        };
        {
            let mut sb = sidebar_state.lock().unwrap_or_else(|p| p.into_inner());
            sb.expanded.insert(format!("conn:{}", id));
            sb.config_connections = connections.clone();
            sb.read_only = connections
                .iter()
                .map(|c| (c.id.clone(), c.read_only))
                .collect();
        }
        let sidebar_nodes = {
            let sb = sidebar_state.lock().unwrap_or_else(|p| p.into_inner());
            build_sidebar_tree(
                &sb.config_connections,
                &id,
                &sb.metadata,
                &sb.expanded,
                &sb.read_only,
            )
        };
        // Update AppState active connection if not already tracked (needed for read-only check).
        let _ = state.conn.active(); // no-op; active is set by controller before event fires
        // clone required: invoke_from_event_loop closure must be 'static
        let _ = slint::invoke_from_event_loop(move || {
            with_ui(&ww, move |ui| {
                let model = Rc::new(slint::VecModel::from(entries));
                ui.set_connection_list(model.into());
                ui.set_active_connection_id(id.into());
                ui.set_conn_safe_dml(safe_dml);
                ui.set_conn_read_only(read_only);
                ui.set_show_connection_form(false);
                // Reopen the DB manager if the form was launched from within it.
                if ui.get_reopen_db_manager_on_form_close() {
                    ui.set_reopen_db_manager_on_form_close(false);
                    ui.set_show_db_manager(true);
                } else {
                    ui.set_show_db_manager(false);
                }
                ui.set_form_testing(false);
                ui.set_form_status("".into());
                ui.set_error_message("".into());
                ui.set_status_connection(status_conn.into());
                ui.set_sidebar_tree(Rc::new(slint::VecModel::from(sidebar_nodes)).into());
                ui.set_sidebar_loading(true);
            });
        });
    }

    fn handle_connection_flags_updated(
        id: String,
        safe_dml: bool,
        read_only: bool,
        ww: slint::Weak<crate::AppWindow>,
        state: SharedState,
        sidebar_state: Arc<Mutex<SidebarUiState>>,
    ) {
        // Update the cached flags and rebuild the sidebar tree outside the UI thread.
        {
            let mut sb = sidebar_state.lock().unwrap_or_else(|p| p.into_inner());
            sb.read_only.insert(id.clone(), read_only);
            if let Some(cc) = sb.config_connections.iter_mut().find(|c| c.id == id) {
                cc.safe_dml = safe_dml;
                cc.read_only = read_only;
            }
        }
        let sidebar_nodes = {
            let sb = sidebar_state.lock().unwrap_or_else(|p| p.into_inner());
            let active_id = state
                .conn
                .active()
                .map(|c| c.id.clone())
                .unwrap_or_default();
            build_sidebar_tree(
                &sb.config_connections,
                &active_id,
                &sb.metadata,
                &sb.expanded,
                &sb.read_only,
            )
        };
        // Recompute the status bar label only when this is the active connection.
        let active_id = state
            .conn
            .active()
            .map(|c| c.id.clone())
            .unwrap_or_default();
        let is_active = active_id == id;
        let new_status = if is_active {
            let base = {
                let sb = sidebar_state.lock().unwrap_or_else(|p| p.into_inner());
                sb.config_connections
                    .iter()
                    .find(|c| c.id == id)
                    .map(|c| match c.database.as_deref() {
                        Some(db) if !db.is_empty() => format!("{} / {}", c.name, db),
                        _ => c.name.clone(),
                    })
                    .unwrap_or_else(|| id.clone())
            };
            if read_only {
                Some(format!("{} · {}", base, t!("status.read_only")))
            } else {
                Some(base)
            }
        } else {
            None
        };
        // clone required: invoke_from_event_loop closure must be 'static
        let _ = slint::invoke_from_event_loop(move || {
            with_ui(&ww, move |ui| {
                ui.set_sidebar_tree(Rc::new(slint::VecModel::from(sidebar_nodes)).into());
                if is_active {
                    ui.set_conn_safe_dml(safe_dml);
                    ui.set_conn_read_only(read_only);
                    if let Some(s) = new_status {
                        ui.set_status_connection(s.into());
                    }
                }
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
                ui.set_status_message(t!("status.connect_failed", msg = msg).to_string().into());
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
                ui.set_status_message(t!("status.running").to_string().into());
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
                ui.set_status_message(
                    t!("status.query_finished", ms = exec_ms, rows = row_count)
                        .to_string()
                        .into(),
                );
                ui.set_result_panel_open(true);
            });
        });
    }

    fn handle_query_cancelled(ww: slint::Weak<crate::AppWindow>) {
        // clone required: invoke_from_event_loop closure must be 'static
        let _ = slint::invoke_from_event_loop(move || {
            with_ui(&ww, |ui| {
                ui.set_is_loading(false);
                ui.set_status_message(t!("status.cancelled").to_string().into());
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
                ui.set_status_message(t!("status.error", msg = summary).to_string().into());
                ui.set_result_panel_open(true);
            });
        });
    }

    fn handle_disconnected(id: String, ww: slint::Weak<crate::AppWindow>) {
        // clone required: invoke_from_event_loop closure must be 'static
        let _ = slint::invoke_from_event_loop(move || {
            with_ui(&ww, move |ui| {
                ui.set_status_message(t!("status.disconnected", id = id).to_string().into());
                ui.set_status_connection(t!("status.not_connected").to_string().into());
            });
        });
    }

    fn handle_connection_removed(
        id: String,
        ww: slint::Weak<crate::AppWindow>,
        _state: SharedState,
        sidebar_state: Arc<Mutex<SidebarUiState>>,
    ) {
        {
            let mut sb = sidebar_state.lock().unwrap_or_else(|p| p.into_inner());
            sb.config_connections.retain(|c| c.id != id);
            sb.read_only.remove(&id);
            sb.metadata.remove(&id);
            sb.expanded.remove(&format!("conn:{}", id));
        }
        let (entries, sidebar_nodes) = {
            let sb = sidebar_state.lock().unwrap_or_else(|p| p.into_inner());
            let e = sb
                .config_connections
                .iter()
                .map(|c| crate::ConnectionEntry {
                    is_active: false,
                    db_type: db_type_label_config(&c.db_type).into(),
                    name: c.name.clone().into(),
                    id: c.id.clone().into(),
                })
                .collect::<Vec<_>>();
            let nodes = build_sidebar_tree(
                &sb.config_connections,
                "",
                &sb.metadata,
                &sb.expanded,
                &sb.read_only,
            );
            (e, nodes)
        };
        // clone required: invoke_from_event_loop closure must be 'static
        let _ = slint::invoke_from_event_loop(move || {
            with_ui(&ww, move |ui| {
                let model = Rc::new(slint::VecModel::from(entries));
                ui.set_connection_list(model.into());
                ui.set_active_connection_id("".into());
                ui.set_conn_read_only(false);
                ui.set_status_connection(t!("status.not_connected").to_string().into());
                ui.set_status_message(t!("status.disconnected", id = id).to_string().into());
                ui.set_sidebar_tree(Rc::new(slint::VecModel::from(sidebar_nodes)).into());
                ui.set_show_db_manager(true);
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
            let active_id = state
                .conn
                .active()
                .map(|c| c.id.clone())
                .unwrap_or_default();
            build_sidebar_tree(
                &sb.config_connections,
                &active_id,
                &sb.metadata,
                &sb.expanded,
                &sb.read_only,
            )
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
                ui.set_status_message(
                    t!("status.metadata_unavailable", msg = msg)
                        .to_string()
                        .into(),
                );
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

    fn handle_ddl_loaded(_tab_id: String, ddl: String, ww: slint::Weak<crate::AppWindow>) {
        // clone required: invoke_from_event_loop closure must be 'static
        let _ = slint::invoke_from_event_loop(move || {
            with_ui(&ww, move |ui| {
                ui.set_tv_ddl(ddl.into());
                ui.set_tv_ddl_loading(false);
            });
        });
    }

    fn handle_ddl_fetch_failed(_tab_id: String, msg: String, ww: slint::Weak<crate::AppWindow>) {
        // clone required: invoke_from_event_loop closure must be 'static
        let _ = slint::invoke_from_event_loop(move || {
            with_ui(&ww, move |ui| {
                ui.set_tv_ddl(format!("Error: {}", msg).into());
                ui.set_tv_ddl_loading(false);
            });
        });
    }

    fn handle_table_data_loaded(
        _tab_id: String,
        result: QueryResult,
        ww: slint::Weak<crate::AppWindow>,
    ) {
        let col_count = result.columns.len();
        let columns: Vec<slint::SharedString> =
            result.columns.iter().map(|c| c.clone().into()).collect();
        let raw_rows: Vec<Vec<Option<String>>> = result.rows.iter().map(|r| r.to_vec()).collect();
        let row_count = result.row_count as i32;
        // clone required: invoke_from_event_loop closure must be 'static
        let _ = slint::invoke_from_event_loop(move || {
            with_ui(&ww, move |ui| {
                ui.set_tv_data_loading(false);
                ui.set_tv_data_error("".into());
                let col_model = Rc::new(slint::VecModel::from(columns));
                ui.set_result_columns(col_model.into());
                let rows: Vec<crate::RowData> = raw_rows.into_iter().map(rows_to_ui).collect();
                ui.set_result_rows(Rc::new(slint::VecModel::from(rows)).into());
                ui.set_result_row_count(row_count);
                let widths: Vec<f32> = vec![DEFAULT_COLUMN_WIDTH; col_count];
                let total_w = col_count as f32 * DEFAULT_COLUMN_WIDTH;
                ui.set_result_col_widths(Rc::new(slint::VecModel::from(widths)).into());
                ui.set_result_total_col_width(total_w);
            });
        });
    }

    fn handle_table_data_failed(_tab_id: String, msg: String, ww: slint::Weak<crate::AppWindow>) {
        // clone required: invoke_from_event_loop closure must be 'static
        let _ = slint::invoke_from_event_loop(move || {
            with_ui(&ww, move |ui| {
                ui.set_tv_data_loading(false);
                ui.set_tv_data_error(msg.into());
            });
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

    fn register_reduce_motion_callback(window: &crate::AppWindow, tx_cmd: mpsc::Sender<Command>) {
        let ui = window.global::<crate::UiState>();
        let window_weak = window.as_weak(); // clone required: on_toggle_reduce_motion closure
        ui.on_toggle_reduce_motion(move || {
            with_ui(&window_weak, |ui| {
                let new_val = !ui.get_reduce_motion();
                ui.set_reduce_motion(new_val);
                send_cmd(
                    &tx_cmd,
                    Command::UpdateConfig(ConfigUpdate::ReduceMotion(new_val)),
                );
            });
        });
    }

    // ── Window lifecycle ──────────────────────────────────────────────────────

    fn register_close_handler(
        window: &crate::AppWindow,
        tabs_state: Rc<RefCell<tabs_state::TabsState>>,
        session_svc: SessionService,
    ) {
        let window_weak = window.as_weak(); // clone required: on_close_requested closure
        let handle = tokio::runtime::Handle::current();
        window.window().on_close_requested(move || {
            let text = window_weak
                .upgrade()
                .map(|w| w.global::<crate::UiState>().get_editor_text().to_string())
                .unwrap_or_default();
            // Flush the active editor text into the tab before persisting.
            tabs_state.borrow_mut().save_current_text(&text);
            let (active_sql_idx, entries) = tabs_state.borrow().session_entries();
            handle.block_on(async {
                if let Err(e) = session_svc.save_tabs(active_sql_idx, &entries).await {
                    tracing::warn!(error = %e, "failed to save session tabs on close");
                }
                if let Err(e) = session_svc.save_last_query(&text).await {
                    tracing::warn!(error = %e, "failed to save last_query on close");
                }
            });
            slint::CloseRequestResponse::HideWindow
        });
    }

    // ── Tab callbacks ─────────────────────────────────────────────────────────

    fn register_tab_callbacks(
        window: &crate::AppWindow,
        tx_cmd: mpsc::Sender<Command>,
        tabs_state: Rc<RefCell<tabs_state::TabsState>>,
        sidebar_state: Arc<Mutex<SidebarUiState>>,
    ) {
        let ui = window.global::<crate::UiState>();

        // on_new_tab: save current editor text, add a SQL Editor tab, switch to it.
        {
            let window_weak = window.as_weak();
            let tabs_state = Rc::clone(&tabs_state); // clone required: callback closure needs owned tabs_state
            ui.on_new_tab(move || {
                let current_text = window_weak
                    .upgrade()
                    .map(|w| w.global::<crate::UiState>().get_editor_text().to_string())
                    .unwrap_or_default();
                let current_sub_tab = window_weak
                    .upgrade()
                    .map(|w| w.global::<crate::UiState>().get_tv_sub_tab() as usize)
                    .unwrap_or(0);
                let (slint_tabs, active_idx) = {
                    let mut ts = tabs_state.borrow_mut();
                    ts.save_current_text(&current_text);
                    ts.save_tv_sub_tab(current_sub_tab);
                    ts.add_sql_editor();
                    (tabs_to_slint(&ts.tabs), ts.active_index as i32)
                };
                with_ui(&window_weak, |ui| {
                    ui.set_tabs(Rc::new(slint::VecModel::from(slint_tabs)).into());
                    ui.set_active_tab_index(active_idx);
                    ui.set_active_tab_kind_sql(true);
                    ui.set_editor_text("".into());
                });
            });
        }

        // on_switch_tab: save current text, switch active tab, restore tab content.
        {
            let window_weak = window.as_weak();
            let tabs_state = Rc::clone(&tabs_state); // clone required: callback closure needs owned tabs_state
            let sidebar_state = Arc::clone(&sidebar_state); // clone required: callback closure needs owned sidebar_state
            ui.on_switch_tab(move |i| {
                let i = i as usize;
                let current_text = window_weak
                    .upgrade()
                    .map(|w| w.global::<crate::UiState>().get_editor_text().to_string())
                    .unwrap_or_default();
                let current_sub_tab = window_weak
                    .upgrade()
                    .map(|w| w.global::<crate::UiState>().get_tv_sub_tab() as usize)
                    .unwrap_or(0);
                let (slint_tabs, active_idx, kind_sql, editor_text, tv_name, tv_cols, tv_sub_tab) = {
                    let mut ts = tabs_state.borrow_mut();
                    ts.save_current_text(&current_text);
                    ts.save_tv_sub_tab(current_sub_tab);
                    ts.set_active(i);
                    let slint_tabs = tabs_to_slint(&ts.tabs);
                    let active_idx = ts.active_index as i32;
                    match ts.active_tab().map(|t| t.kind.clone()) {
                        Some(tabs_state::TabKind::SqlEditor { query_text }) => (
                            slint_tabs,
                            active_idx,
                            true,
                            query_text,
                            String::new(),
                            vec![],
                            0usize,
                        ),
                        Some(tabs_state::TabKind::TableView {
                            conn_id,
                            table_name,
                            sub_tab,
                        }) => {
                            let sb = sidebar_state.lock().unwrap_or_else(|p| p.into_inner());
                            let cols = sb
                                .metadata
                                .get(&conn_id)
                                .and_then(|meta| {
                                    meta.tables
                                        .iter()
                                        .chain(meta.views.iter())
                                        .find(|t| t.name == table_name)
                                        .map(|ti| columns_to_slint(&ti.columns))
                                })
                                .unwrap_or_default();
                            (
                                slint_tabs,
                                active_idx,
                                false,
                                String::new(),
                                table_name,
                                cols,
                                sub_tab,
                            )
                        }
                        None => (
                            slint_tabs,
                            active_idx,
                            true,
                            String::new(),
                            String::new(),
                            vec![],
                            0usize,
                        ),
                    }
                };
                with_ui(&window_weak, |ui| {
                    ui.set_tabs(Rc::new(slint::VecModel::from(slint_tabs)).into());
                    ui.set_active_tab_index(active_idx);
                    ui.set_active_tab_kind_sql(kind_sql);
                    if kind_sql {
                        ui.set_editor_text(editor_text.into());
                    } else {
                        ui.set_tv_table_name(tv_name.into());
                        ui.set_tv_columns(Rc::new(slint::VecModel::from(tv_cols)).into());
                        ui.set_tv_sub_tab(tv_sub_tab as i32);
                    }
                });
            });
        }

        // on_close_tab: close the given tab; prevents closing the last SQL Editor tab.
        {
            let window_weak = window.as_weak();
            let tabs_state = Rc::clone(&tabs_state); // clone required: callback closure needs owned tabs_state
            ui.on_close_tab(move |i| {
                let i = i as usize;
                let current_text = window_weak
                    .upgrade()
                    .map(|w| w.global::<crate::UiState>().get_editor_text().to_string())
                    .unwrap_or_default();
                let mut ts = tabs_state.borrow_mut();
                if ts.active_index != i {
                    ts.save_current_text(&current_text);
                }
                if !ts.close(i) {
                    return; // can't close the last SQL Editor tab
                }
                let slint_tabs = tabs_to_slint(&ts.tabs);
                let active_idx = ts.active_index as i32;
                let (kind_sql, editor_text) = match ts.active_tab().map(|t| t.kind.clone()) {
                    Some(tabs_state::TabKind::SqlEditor { query_text }) => (true, query_text),
                    _ => (false, String::new()),
                };
                drop(ts);
                with_ui(&window_weak, |ui| {
                    ui.set_tabs(Rc::new(slint::VecModel::from(slint_tabs)).into());
                    ui.set_active_tab_index(active_idx);
                    ui.set_active_tab_kind_sql(kind_sql);
                    if kind_sql {
                        ui.set_editor_text(editor_text.into());
                    }
                });
            });
        }

        // on_copy_tv_ddl: write the DDL text to the system clipboard.
        {
            let window_weak = window.as_weak();
            ui.on_copy_tv_ddl(move || {
                let ddl = window_weak
                    .upgrade()
                    .map(|w| w.global::<crate::UiState>().get_tv_ddl().to_string())
                    .unwrap_or_default();
                if let Ok(mut clip) = arboard::Clipboard::new() {
                    let _ = clip.set_text(ddl);
                }
            });
        }

        // on_refresh_tv_data: re-fetch the table data for the active Table View tab.
        {
            let window_weak = window.as_weak();
            let tx_cmd = tx_cmd.clone(); // clone required: callback closure needs owned tx_cmd
            let tabs_state = Rc::clone(&tabs_state); // clone required: callback closure needs owned tabs_state
            ui.on_refresh_tv_data(move || {
                let Some(w) = window_weak.upgrade() else {
                    return;
                };
                let ui = w.global::<crate::UiState>();
                let tv_table_name = ui.get_tv_table_name().to_string();
                let conn_id = ui.get_active_connection_id().to_string();
                let page_size = ui.get_tv_page_size() as usize;
                if conn_id.is_empty() || tv_table_name.is_empty() {
                    return;
                }
                let tab_id = {
                    let ts = tabs_state.borrow();
                    ts.find_table_view(&conn_id, &tv_table_name)
                        .and_then(|idx| ts.tabs.get(idx))
                        .map(|t| t.id.clone())
                        .unwrap_or_default()
                };
                if tab_id.is_empty() {
                    return;
                }
                with_ui(&window_weak, |ui| ui.set_tv_data_loading(true));
                send_cmd(
                    &tx_cmd,
                    Command::FetchTableData {
                        tab_id,
                        conn_id,
                        table_name: tv_table_name,
                        page_size,
                    },
                );
            });
        }

        // on_fetch_tv_ddl: fetch the DDL statement for the active Table View tab.
        {
            let window_weak = window.as_weak();
            let tx_cmd = tx_cmd.clone(); // clone required: callback closure needs owned tx_cmd
            let tabs_state = Rc::clone(&tabs_state); // clone required: callback closure needs owned tabs_state
            let sidebar_state = Arc::clone(&sidebar_state); // clone required: callback closure needs owned sidebar_state
            ui.on_fetch_tv_ddl(move || {
                let Some(w) = window_weak.upgrade() else {
                    return;
                };
                let ui = w.global::<crate::UiState>();
                let tv_table_name = ui.get_tv_table_name().to_string();
                let conn_id = ui.get_active_connection_id().to_string();
                if conn_id.is_empty() || tv_table_name.is_empty() {
                    return;
                }
                let tab_id = {
                    let ts = tabs_state.borrow();
                    ts.find_table_view(&conn_id, &tv_table_name)
                        .and_then(|idx| ts.tabs.get(idx))
                        .map(|t| t.id.clone())
                        .unwrap_or_default()
                };
                if tab_id.is_empty() {
                    return;
                }
                let kind = {
                    let sb = sidebar_state.lock().unwrap_or_else(|p| p.into_inner());
                    if let Some(meta) = sb.metadata.get(&conn_id) {
                        if meta.views.iter().any(|v| v.name == tv_table_name) {
                            "view".to_string()
                        } else {
                            "table".to_string()
                        }
                    } else {
                        "table".to_string()
                    }
                };
                with_ui(&window_weak, |ui| {
                    ui.set_tv_ddl("".into());
                    ui.set_tv_ddl_loading(true);
                });
                send_cmd(
                    &tx_cmd,
                    Command::FetchDdl {
                        tab_id,
                        conn_id,
                        name: tv_table_name,
                        kind,
                    },
                );
            });
        }

        // on_change_tv_page_size: re-fetch table data with a new row limit.
        {
            let window_weak = window.as_weak();
            let tx_cmd = tx_cmd.clone(); // clone required: callback closure needs owned tx_cmd
            let tabs_state = Rc::clone(&tabs_state); // clone required: callback closure needs owned tabs_state
            ui.on_change_tv_page_size(move |n| {
                let Some(w) = window_weak.upgrade() else {
                    return;
                };
                let ui = w.global::<crate::UiState>();
                let page_size = n as usize;
                ui.set_tv_page_size(n);
                let tv_table_name = ui.get_tv_table_name().to_string();
                let conn_id = ui.get_active_connection_id().to_string();
                if conn_id.is_empty() || tv_table_name.is_empty() {
                    return;
                }
                let tab_id = {
                    let ts = tabs_state.borrow();
                    ts.find_table_view(&conn_id, &tv_table_name)
                        .and_then(|idx| ts.tabs.get(idx))
                        .map(|t| t.id.clone())
                        .unwrap_or_default()
                };
                if tab_id.is_empty() {
                    return;
                }
                with_ui(&window_weak, |ui| ui.set_tv_data_loading(true));
                send_cmd(
                    &tx_cmd,
                    Command::FetchTableData {
                        tab_id,
                        conn_id,
                        table_name: tv_table_name,
                        page_size,
                    },
                );
            });
        }
    }

    // ── Menu bar callbacks ────────────────────────────────────────────────────

    fn register_menu_callbacks(window: &crate::AppWindow, tx_cmd: mpsc::Sender<Command>) {
        let ui = window.global::<crate::UiState>();

        // quit: exit the event loop (closes the application)
        ui.on_quit(|| {
            let _ = slint::quit_event_loop();
        });

        // run-all: execute the entire editor content
        {
            let tx_cmd = tx_cmd.clone(); // clone required: callback closure needs owned tx_cmd
            let window_weak = window.as_weak(); // clone required: check_safe_dml needs window ref
            ui.on_run_all(move |sql| {
                if check_safe_dml(&window_weak, &sql, "all") {
                    return;
                }
                send_cmd(&tx_cmd, Command::RunAll(sql.to_string()));
            });
        }
    }

    // ── Sidebar callbacks ─────────────────────────────────────────────────────

    fn register_sidebar_callbacks(
        window: &crate::AppWindow,
        state: SharedState,
        tx_cmd: mpsc::Sender<Command>,
        sidebar_state: Arc<Mutex<SidebarUiState>>,
        enc_key: [u8; 32],
        tabs_state: Rc<RefCell<tabs_state::TabsState>>,
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
                    ui.set_form_edit_id("".into());
                    ui.set_form_safe_dml(true);
                    ui.set_form_read_only(false);
                    ui.set_show_connection_form(true);
                });
            });
        }

        // edit-connection: open the connection form pre-filled for an existing connection.
        // Both tabs are filled: whichever tab was NOT used to save the connection is
        // derived via derive_conn_string / parse_conn_string.
        {
            let window_weak = window.as_weak();
            let sidebar_state = Arc::clone(&sidebar_state);
            ui_state.on_edit_connection(move |id| {
                let id = id.to_string();
                // Look up from config_connections — works even when the DB is not running.
                let conn_cfg = {
                    let sb = sidebar_state.lock().unwrap_or_else(|p| p.into_inner());
                    sb.config_connections.iter().find(|c| c.id == id).cloned()
                };
                let Some(conn_cfg) = conn_cfg else {
                    return;
                };
                let conn = config_to_db_conn(&conn_cfg);
                let safe_dml = conn_cfg.safe_dml;
                let read_only = conn_cfg.read_only;

                // Decrypt stored password (only present for individual-field connections).
                let stored_password = conn
                    .password_encrypted
                    .as_ref()
                    .and_then(|enc| crypto::decrypt(enc, &enc_key).ok())
                    .unwrap_or_default();

                let is_conn_string = conn.connection_string.is_some();
                let db_type_idx: i32 = match conn.db_type {
                    DbType::PostgreSQL => 0,
                    DbType::MySQL => 1,
                    DbType::SQLite => 2,
                };

                // Derive the connection string from individual fields (or use the stored one).
                let conn_string = if is_conn_string {
                    conn.connection_string.clone().unwrap_or_default()
                } else {
                    derive_conn_string(
                        &conn.db_type,
                        conn.host.as_deref().unwrap_or(""),
                        conn.port,
                        conn.user.as_deref().unwrap_or(""),
                        &stored_password,
                        conn.database.as_deref().unwrap_or(""),
                    )
                };

                // Parse individual fields from the connection string (or use the stored ones).
                let (host, port, user, field_password, database) = if is_conn_string {
                    parse_conn_string(&conn_string, &conn.db_type).unwrap_or_default()
                } else {
                    (
                        conn.host.clone().unwrap_or_default(),
                        conn.port,
                        conn.user.clone().unwrap_or_default(),
                        stored_password,
                        conn.database.clone().unwrap_or_default(),
                    )
                };

                with_ui(&window_weak, move |ui| {
                    ui.set_form_edit_id(conn.id.clone().into());
                    ui.set_form_name(conn.name.clone().into());
                    ui.set_form_db_type(db_type_idx);
                    ui.set_form_tab_index(if is_conn_string { 0 } else { 1 });
                    ui.set_form_conn_string(conn_string.into());
                    ui.set_form_host(host.into());
                    ui.set_form_port(port.map(|p| p.to_string()).unwrap_or_default().into());
                    ui.set_form_user(user.into());
                    ui.set_form_password(field_password.into());
                    ui.set_form_database(database.into());
                    ui.set_form_status("".into());
                    ui.set_form_testing(false);
                    ui.set_form_test_ok(false);
                    ui.set_form_safe_dml(safe_dml);
                    ui.set_form_read_only(read_only);
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
                        let conn_cfg = {
                            let sb = sidebar_state.lock().unwrap_or_else(|p| p.into_inner());
                            sb.config_connections
                                .iter()
                                .find(|c| c.id == conn_id)
                                .cloned()
                        };
                        if let Some(cc) = conn_cfg {
                            let conn = config_to_db_conn(&cc);
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
                    let active_id = state
                        .conn
                        .active()
                        .map(|c| c.id.clone())
                        .unwrap_or_default();
                    build_sidebar_tree(
                        &sb.config_connections,
                        &active_id,
                        &sb.metadata,
                        &sb.expanded,
                        &sb.read_only,
                    )
                };
                with_ui(&window_weak, |ui| {
                    let model = ui.get_sidebar_tree();
                    if model.row_count() == nodes.len() {
                        for (i, node) in nodes.into_iter().enumerate() {
                            model.set_row_data(i, node);
                        }
                    } else {
                        ui.set_sidebar_tree(Rc::new(slint::VecModel::from(nodes)).into());
                    }
                });
            });
        }

        // connect-db: connect to a saved connection from the DB tab by id.
        // Mirrors the connection-switching logic in toggle-sidebar-node.
        {
            // clone required: callback closure needs owned captures
            let tx_cmd = tx_cmd.clone();
            let state = state.clone();
            let sidebar_state = Arc::clone(&sidebar_state); // clone required: lookup from config_connections
            ui_state.on_connect_db(move |id| {
                let id = id.to_string();
                let active_id = state
                    .conn
                    .active()
                    .map(|c| c.id.clone())
                    .unwrap_or_default();
                if id == active_id {
                    return;
                }
                let conn_cfg = {
                    let sb = sidebar_state.lock().unwrap_or_else(|p| p.into_inner());
                    sb.config_connections.iter().find(|c| c.id == id).cloned()
                };
                if let Some(cc) = conn_cfg {
                    let conn = config_to_db_conn(&cc);
                    let password = conn
                        .password_encrypted
                        .as_ref()
                        .and_then(|enc| crypto::decrypt(enc, &enc_key).ok());
                    send_cmd(&tx_cmd, Command::Connect(conn, password));
                }
            });
        }

        // open-db-manager: show the DB manager dialog.
        {
            let window_weak = window.as_weak();
            ui_state.on_open_db_manager(move || {
                with_ui(&window_weak, |ui| ui.set_show_db_manager(true));
            });
        }

        // disconnect: disconnect the active connection by id.
        // Clears AppState active_id, collapses the sidebar node, removes cached
        // metadata for that connection, and rebuilds the sidebar tree immediately.
        {
            let tx_cmd = tx_cmd.clone(); // clone required: callback closure needs owned tx_cmd
            let state = state.clone();
            let sidebar_state = Arc::clone(&sidebar_state);
            let window_weak = window.as_weak();
            ui_state.on_disconnect(move |id| {
                let id = id.to_string();
                if id.is_empty() {
                    return;
                }
                // Clear active connection in AppState so toggle-sidebar-node can
                // reconnect when the collapsed node is clicked again.
                state.conn.clear_active();

                // Collapse the node and drop metadata for the disconnected connection.
                {
                    let mut sb = sidebar_state.lock().unwrap_or_else(|p| p.into_inner());
                    sb.expanded.remove(&format!("conn:{}", id));
                    sb.metadata.remove(&id);
                }

                // Rebuild tree (no active connection, no expanded node for id).
                let (nodes, entries) = {
                    let sb = sidebar_state.lock().unwrap_or_else(|p| p.into_inner());
                    let nodes = build_sidebar_tree(
                        &sb.config_connections,
                        "",
                        &sb.metadata,
                        &sb.expanded,
                        &sb.read_only,
                    );
                    let entries = sb
                        .config_connections
                        .iter()
                        .map(|c| crate::ConnectionEntry {
                            is_active: false,
                            db_type: db_type_label_config(&c.db_type).into(),
                            name: c.name.clone().into(),
                            id: c.id.clone().into(),
                        })
                        .collect::<Vec<_>>();
                    (nodes, entries)
                };

                // Already on the UI thread — update directly.
                with_ui(&window_weak, move |ui| {
                    ui.set_active_connection_id("".into());
                    ui.set_connection_list(Rc::new(slint::VecModel::from(entries)).into());
                    ui.set_sidebar_tree(Rc::new(slint::VecModel::from(nodes)).into());
                    ui.set_sidebar_loading(false);
                });

                send_cmd(&tx_cmd, Command::Disconnect(id));
            });
        }

        // table-double-clicked: open a Table View tab for the clicked table/view.
        // Saves the active editor text, opens or focuses the TV tab, then triggers
        // a FetchTableData command if the tab is new.
        {
            let window_weak = window.as_weak();
            let tx_cmd = tx_cmd.clone(); // clone required: callback closure needs owned tx_cmd
            let tabs_state = Rc::clone(&tabs_state); // clone required: callback closure needs owned tabs_state
            let sidebar_state = Arc::clone(&sidebar_state); // clone required: callback closure needs owned sidebar_state
            ui_state.on_table_double_clicked(move |name| {
                let name = name.to_string();
                let (conn_id, current_text, current_sub_tab) = {
                    let Some(w) = window_weak.upgrade() else {
                        return;
                    };
                    let ui = w.global::<crate::UiState>();
                    (
                        ui.get_active_connection_id().to_string(),
                        ui.get_editor_text().to_string(),
                        ui.get_tv_sub_tab() as usize,
                    )
                };
                if conn_id.is_empty() {
                    return;
                }
                let (tab_id, is_new, slint_tabs, active_idx, tv_sub_tab) = {
                    let mut ts = tabs_state.borrow_mut();
                    ts.save_current_text(&current_text);
                    ts.save_tv_sub_tab(current_sub_tab);
                    let (tab_id, _idx, is_new) = ts.open_table_view(&conn_id, &name);
                    let tv_sub_tab = ts.active_tv_sub_tab();
                    let slint_tabs = tabs_to_slint(&ts.tabs);
                    let active_idx = ts.active_index as i32;
                    (tab_id, is_new, slint_tabs, active_idx, tv_sub_tab)
                };
                let tv_cols = {
                    let sb = sidebar_state.lock().unwrap_or_else(|p| p.into_inner());
                    sb.metadata
                        .get(&conn_id)
                        .and_then(|meta| {
                            meta.tables
                                .iter()
                                .chain(meta.views.iter())
                                .find(|t| t.name == name)
                                .map(|ti| columns_to_slint(&ti.columns))
                        })
                        .unwrap_or_default()
                };
                with_ui(&window_weak, |ui| {
                    ui.set_tabs(Rc::new(slint::VecModel::from(slint_tabs)).into());
                    ui.set_active_tab_index(active_idx);
                    ui.set_active_tab_kind_sql(false);
                    ui.set_tv_table_name(name.clone().into());
                    ui.set_tv_sub_tab(tv_sub_tab as i32);
                    if is_new {
                        ui.set_tv_page_size(1000);
                    }
                    ui.set_tv_data_loading(is_new);
                    ui.set_tv_data_error("".into());
                    ui.set_tv_ddl("".into());
                    ui.set_tv_ddl_loading(false);
                    ui.set_tv_columns(Rc::new(slint::VecModel::from(tv_cols)).into());
                });
                if is_new {
                    send_cmd(
                        &tx_cmd,
                        Command::FetchTableData {
                            tab_id,
                            conn_id,
                            table_name: name,
                            page_size: 1000,
                        },
                    );
                }
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
                with_ui(&window_weak, |ui| {
                    ui.set_show_connection_form(false);
                    if ui.get_reopen_db_manager_on_form_close() {
                        ui.set_reopen_db_manager_on_form_close(false);
                        ui.set_show_db_manager(true);
                    }
                });
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
                        let conn_id = conn.id.clone();
                        let safe_dml = ui.get_form_safe_dml();
                        let read_only = ui.get_form_read_only();
                        send_cmd(&tx_cmd, Command::Connect(conn, password));
                        send_cmd(
                            &tx_cmd,
                            Command::UpdateConfig(ConfigUpdate::ConnectionFlags {
                                id: conn_id,
                                safe_dml,
                                read_only,
                            }),
                        );
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
                    let conn_id = conn.id.clone();
                    let safe_dml = ui.get_form_safe_dml();
                    let read_only = ui.get_form_read_only();
                    send_cmd(&tx_cmd, Command::Connect(conn, password));
                    send_cmd(
                        &tx_cmd,
                        Command::UpdateConfig(ConfigUpdate::ConnectionFlags {
                            id: conn_id,
                            safe_dml,
                            read_only,
                        }),
                    );
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

        // delete-connection: remove from config, disconnect if active
        {
            let window_weak = window.as_weak();
            // clone required: callback closure needs owned tx_cmd
            let tx_cmd = tx_cmd.clone();
            ui_state.on_delete_connection(move || {
                with_ui(&window_weak, |ui| {
                    let id = ui.get_form_edit_id().to_string();
                    if !id.is_empty() {
                        send_cmd(&tx_cmd, Command::RemoveConnection(id));
                        ui.set_show_connection_form(false);
                    }
                });
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
                    None => s.len() as i32, // Extend to end of text on last line.
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
            let window_weak = window.as_weak(); // clone required: check_safe_dml needs window ref
            ui.on_run_query(move |sql| {
                if check_safe_dml(&window_weak, &sql, "query") {
                    return;
                }
                send_cmd(&tx_cmd, Command::RunQuery(sql.to_string()));
            });
        }
        {
            let tx_cmd = tx_cmd.clone(); // clone required: callback closure needs owned tx_cmd
            let window_weak = window.as_weak(); // clone required: check_safe_dml needs window ref
            ui.on_run_query_at_cursor(move |sql, cursor| {
                let stmt = extract_statement_at(sql.as_str(), cursor as usize);
                if !stmt.is_empty() {
                    if check_safe_dml(&window_weak, stmt, "cursor") {
                        return;
                    }
                    send_cmd(&tx_cmd, Command::RunQuery(stmt.to_owned()));
                }
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
    const INSERT_SQL_DEFAULT_FILENAME: &str = "query_result.sql";

    fn register_export_callbacks(
        window: &crate::AppWindow,
        original_data: SharedOriginalData,
        state: SharedState,
    ) {
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

        // ── INSERT SQL export ─────────────────────────────────────────────────
        {
            let window_weak = window.as_weak(); // clone required: on_export_insert_sql closure
            let original_data = Arc::clone(&original_data); // clone required: on_export_insert_sql closure
            ui.on_export_insert_sql(move || {
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
                // Auto-detect table name from last SQL; fall back to a safe default.
                let table_name = state
                    .query
                    .last_sql()
                    .as_deref()
                    .and_then(wf_query::analyzer::extract_single_table_name)
                    .unwrap_or_else(|| "exported_table".to_string());
                let window_weak = window_weak.clone(); // clone required: tokio::spawn needs 'static
                tokio::spawn(async move {
                    let Some(handle) = rfd::AsyncFileDialog::new()
                        .set_title("Save Insert SQL")
                        .set_file_name(Self::INSERT_SQL_DEFAULT_FILENAME)
                        .add_filter("SQL files", &["sql"])
                        .save_file()
                        .await
                    else {
                        return; // user cancelled
                    };
                    let path = handle.path().to_path_buf();
                    let result =
                        wf_query::export::export_insert_sql(&columns, &rows, &table_name, &path);
                    let msg = match result {
                        Ok(()) => format!("Saved Insert SQL: {}", path.display()),
                        Err(e) => format!("Insert SQL export failed: {e}"),
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

        // confirm-safe-dml: user confirmed execution of dangerous DML.
        {
            // clone required: callback closure must be 'static
            let window_weak = window_weak.clone();
            let tx_cmd = tx_cmd.clone();
            ui_state.on_confirm_safe_dml(move || {
                with_ui(&window_weak, |ui| {
                    let sql = ui.get_safe_dml_pending_sql().to_string();
                    let kind = ui.get_safe_dml_pending_kind().to_string();
                    ui.set_show_safe_dml_confirm(false);
                    ui.set_safe_dml_pending_sql("".into());
                    let cmd = match kind.as_str() {
                        "all" => Command::RunAll(sql),
                        _ => Command::RunQuery(sql),
                    };
                    send_cmd(&tx_cmd, cmd);
                });
            });
        }

        // dismiss-safe-dml-confirm: user cancelled the dangerous DML popup.
        {
            // clone required: callback closure must be 'static
            let window_weak = window_weak.clone();
            ui_state.on_dismiss_safe_dml_confirm(move || {
                with_ui(&window_weak, |ui| {
                    ui.set_show_safe_dml_confirm(false);
                    ui.set_safe_dml_pending_sql("".into());
                });
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

    // ── Language callback ─────────────────────────────────────────────────────

    fn register_language_callback(window: &crate::AppWindow, tx_cmd: mpsc::Sender<Command>) {
        let ui = window.global::<crate::UiState>();
        let window_weak = window.as_weak(); // clone required: on_set_language closure
        ui.on_set_language(move |lang| {
            let lang = lang.to_string();
            // Immediate locale switch on the UI thread — all @tr() bindings re-evaluate.
            let _ = slint::select_bundled_translation(&lang);
            rust_i18n::set_locale(&lang);
            // Update UiState.language so the checkmarks in the menu update.
            with_ui(&window_weak, |ui| ui.set_language(lang.clone().into()));
            // Persist to config.toml via controller.
            send_cmd(&tx_cmd, Command::UpdateConfig(ConfigUpdate::Language(lang)));
        });
    }

    // ── Find / replace callbacks ──────────────────────────────────────────────

    fn register_find_replace_callbacks(
        window: &crate::AppWindow,
        find_history_svc: FindHistoryService,
    ) {
        let ui_state = window.global::<crate::UiState>();
        let find_state: Rc<RefCell<FindState>> = Rc::new(RefCell::new(FindState::default()));
        let history: SharedHistorySnapshot =
            Arc::new(std::sync::Mutex::new(HistorySnapshot::default()));

        // ── load-find-history: reset nav state and load from SQLite on bar open
        {
            let find_state = find_state.clone(); // clone required: captured by on_load_find_history
            let svc = find_history_svc.clone(); // clone required: moved into tokio::spawn
            let history = history.clone(); // clone required: moved into tokio::spawn
            ui_state.on_load_find_history(move || {
                {
                    let mut state = find_state.borrow_mut();
                    state.find_hist_idx = None;
                    state.replace_hist_idx = None;
                }
                let svc = svc.clone(); // clone required: moved into tokio::spawn
                let history = history.clone(); // clone required: moved into tokio::spawn
                tokio::spawn(async move {
                    let find_items = svc.get("find", 30).await.unwrap_or_default();
                    let replace_items = svc.get("replace", 30).await.unwrap_or_default();
                    let mut snap = history.lock().unwrap_or_else(|p| p.into_inner());
                    snap.find = find_items;
                    snap.replace = replace_items;
                });
            });
        }

        // ── find-search: recompute all matches, go to first ──────────────────
        {
            let find_state = find_state.clone(); // clone required: captured by on_find_search
            let window_weak = window.as_weak();
            ui_state.on_find_search(move || {
                let Some(w) = window_weak.upgrade() else {
                    return;
                };
                let ui = w.global::<crate::UiState>();
                let text = ui.get_editor_text().to_string();
                let query = ui.get_find_query().to_string();
                let cs = ui.get_find_case_sensitive();
                let rx = ui.get_find_use_regex();

                let mut state = find_state.borrow_mut();
                state.find_hist_idx = None; // typing resets history browsing
                state.last_query = String::new(); // force re-computation
                state.update(&text, &query, cs, rx);

                if state.matches.is_empty() {
                    ui.set_find_current_match(0);
                    ui.set_find_total_matches(0);
                    ui.set_find_sel_start(-1);
                    ui.set_find_sel_end(-1);
                    return;
                }
                state.current = 0;
                let (start, end) = state.matches[0];
                ui.set_find_current_match(1);
                ui.set_find_total_matches(state.matches.len() as i32);
                ui.set_find_sel_start(start as i32);
                ui.set_find_sel_end(end as i32);
            });
        }

        // ── find-next ────────────────────────────────────────────────────────
        {
            let find_state = find_state.clone(); // clone required: captured by on_find_next
            let history = history.clone(); // clone required: captured by on_find_next
            let svc = find_history_svc.clone(); // clone required: captured by on_find_next
            let window_weak = window.as_weak();
            ui_state.on_find_next(move || {
                let Some(w) = window_weak.upgrade() else {
                    return;
                };
                let ui = w.global::<crate::UiState>();
                let text = ui.get_editor_text().to_string();
                let query = ui.get_find_query().to_string();
                let cs = ui.get_find_case_sensitive();
                let rx = ui.get_find_use_regex();

                let mut state = find_state.borrow_mut();
                let params_changed = state.params_changed(&query, cs, rx);
                state.update(&text, &query, cs, rx);
                if state.matches.is_empty() {
                    ui.set_find_current_match(0);
                    ui.set_find_total_matches(0);
                    ui.set_find_sel_start(-1);
                    ui.set_find_sel_end(-1);
                    return;
                }
                if !params_changed {
                    state.current = (state.current + 1) % state.matches.len();
                }
                let (start, end) = state.matches[state.current];
                ui.set_find_current_match((state.current + 1) as i32);
                ui.set_find_total_matches(state.matches.len() as i32);
                ui.set_find_sel_start(start as i32);
                ui.set_find_sel_end(end as i32);
                drop(state);
                // Persist the query whenever the user navigates to a result.
                if !query.is_empty() {
                    {
                        let mut snap = history.lock().unwrap_or_else(|p| p.into_inner());
                        if !snap.find.contains(&query) {
                            snap.find.insert(0, query.clone());
                            snap.find.truncate(30);
                        }
                    }
                    let svc = svc.clone(); // clone required: moved into tokio::spawn
                    tokio::spawn(async move {
                        if let Err(e) = svc.save("find", &query).await {
                            tracing::warn!(error = %e, %query, "failed to save find history");
                        }
                    });
                }
            });
        }

        // ── find-prev ────────────────────────────────────────────────────────
        {
            let find_state = find_state.clone(); // clone required: captured by on_find_prev
            let history = history.clone(); // clone required: captured by on_find_prev
            let svc = find_history_svc.clone(); // clone required: captured by on_find_prev
            let window_weak = window.as_weak();
            ui_state.on_find_prev(move || {
                let Some(w) = window_weak.upgrade() else {
                    return;
                };
                let ui = w.global::<crate::UiState>();
                let text = ui.get_editor_text().to_string();
                let query = ui.get_find_query().to_string();
                let cs = ui.get_find_case_sensitive();
                let rx = ui.get_find_use_regex();

                let mut state = find_state.borrow_mut();
                state.update(&text, &query, cs, rx);
                if state.matches.is_empty() {
                    ui.set_find_current_match(0);
                    ui.set_find_total_matches(0);
                    ui.set_find_sel_start(-1);
                    ui.set_find_sel_end(-1);
                    return;
                }
                state.current = if state.current == 0 {
                    state.matches.len() - 1
                } else {
                    state.current - 1
                };
                let (start, end) = state.matches[state.current];
                ui.set_find_current_match((state.current + 1) as i32);
                ui.set_find_total_matches(state.matches.len() as i32);
                ui.set_find_sel_start(start as i32);
                ui.set_find_sel_end(end as i32);
                drop(state);
                // Persist the query whenever the user navigates to a result.
                if !query.is_empty() {
                    {
                        let mut snap = history.lock().unwrap_or_else(|p| p.into_inner());
                        if !snap.find.contains(&query) {
                            snap.find.insert(0, query.clone());
                            snap.find.truncate(30);
                        }
                    }
                    let svc = svc.clone(); // clone required: moved into tokio::spawn
                    tokio::spawn(async move {
                        if let Err(e) = svc.save("find", &query).await {
                            tracing::warn!(error = %e, %query, "failed to save find history");
                        }
                    });
                }
            });
        }

        // ── find-history-prev (↑ in find input) ──────────────────────────────
        {
            let find_state = find_state.clone(); // clone required: captured by on_find_history_prev
            let history = history.clone(); // clone required: captured by on_find_history_prev
            let window_weak = window.as_weak();
            ui_state.on_find_history_prev(move || {
                let Some(w) = window_weak.upgrade() else {
                    return;
                };
                let ui = w.global::<crate::UiState>();
                let snap = history.lock().unwrap_or_else(|p| p.into_inner());
                if snap.find.is_empty() {
                    return;
                }
                let mut state = find_state.borrow_mut();
                let new_idx = match state.find_hist_idx {
                    None => {
                        state.find_draft = ui.get_find_query().to_string();
                        0
                    }
                    Some(i) => (i + 1).min(snap.find.len() - 1),
                };
                state.find_hist_idx = Some(new_idx);
                let query = snap.find[new_idx].clone();
                drop(snap);
                drop(state);

                ui.set_find_query(query.clone().into());
                let text = ui.get_editor_text().to_string();
                let cs = ui.get_find_case_sensitive();
                let rx = ui.get_find_use_regex();
                let mut state = find_state.borrow_mut();
                state.last_query = String::new();
                state.update(&text, &query, cs, rx);
                if state.matches.is_empty() {
                    ui.set_find_current_match(0);
                    ui.set_find_total_matches(0);
                    ui.set_find_sel_start(-1);
                    ui.set_find_sel_end(-1);
                } else {
                    state.current = 0;
                    let (start, end) = state.matches[0];
                    ui.set_find_current_match(1);
                    ui.set_find_total_matches(state.matches.len() as i32);
                    ui.set_find_sel_start(start as i32);
                    ui.set_find_sel_end(end as i32);
                }
            });
        }

        // ── find-history-next (↓ in find input) ──────────────────────────────
        {
            let find_state = find_state.clone(); // clone required: captured by on_find_history_next
            let history = history.clone(); // clone required: captured by on_find_history_next
            let window_weak = window.as_weak();
            ui_state.on_find_history_next(move || {
                let Some(w) = window_weak.upgrade() else {
                    return;
                };
                let ui = w.global::<crate::UiState>();
                let mut state = find_state.borrow_mut();
                match state.find_hist_idx {
                    None => (),
                    Some(0) => {
                        state.find_hist_idx = None;
                        let draft = state.find_draft.clone();
                        drop(state);

                        ui.set_find_query(draft.clone().into());
                        let text = ui.get_editor_text().to_string();
                        let cs = ui.get_find_case_sensitive();
                        let rx = ui.get_find_use_regex();
                        let mut st = find_state.borrow_mut();
                        st.last_query = String::new();
                        st.update(&text, &draft, cs, rx);
                        if st.matches.is_empty() {
                            ui.set_find_current_match(0);
                            ui.set_find_total_matches(0);
                            ui.set_find_sel_start(-1);
                            ui.set_find_sel_end(-1);
                        } else {
                            st.current = 0;
                            let (start, end) = st.matches[0];
                            ui.set_find_current_match(1);
                            ui.set_find_total_matches(st.matches.len() as i32);
                            ui.set_find_sel_start(start as i32);
                            ui.set_find_sel_end(end as i32);
                        }
                    }
                    Some(i) => {
                        let snap = history.lock().unwrap_or_else(|p| p.into_inner());
                        let new_idx = i - 1;
                        state.find_hist_idx = Some(new_idx);
                        let query = snap.find[new_idx].clone();
                        drop(snap);
                        drop(state);

                        ui.set_find_query(query.clone().into());
                        let text = ui.get_editor_text().to_string();
                        let cs = ui.get_find_case_sensitive();
                        let rx = ui.get_find_use_regex();
                        let mut st = find_state.borrow_mut();
                        st.last_query = String::new();
                        st.update(&text, &query, cs, rx);
                        if st.matches.is_empty() {
                            ui.set_find_current_match(0);
                            ui.set_find_total_matches(0);
                            ui.set_find_sel_start(-1);
                            ui.set_find_sel_end(-1);
                        } else {
                            st.current = 0;
                            let (start, end) = st.matches[0];
                            ui.set_find_current_match(1);
                            ui.set_find_total_matches(st.matches.len() as i32);
                            ui.set_find_sel_start(start as i32);
                            ui.set_find_sel_end(end as i32);
                        }
                    }
                }
            });
        }

        // ── replace-history-prev (↑ in replace input) ────────────────────────
        {
            let find_state = find_state.clone(); // clone required: captured by on_replace_history_prev
            let history = history.clone(); // clone required: captured by on_replace_history_prev
            let window_weak = window.as_weak();
            ui_state.on_replace_history_prev(move || {
                let Some(w) = window_weak.upgrade() else {
                    return;
                };
                let ui = w.global::<crate::UiState>();
                let snap = history.lock().unwrap_or_else(|p| p.into_inner());
                if snap.replace.is_empty() {
                    return;
                }
                let mut state = find_state.borrow_mut();
                let new_idx = match state.replace_hist_idx {
                    None => {
                        state.replace_draft = ui.get_find_replace_text().to_string();
                        0
                    }
                    Some(i) => (i + 1).min(snap.replace.len() - 1),
                };
                state.replace_hist_idx = Some(new_idx);
                let text = snap.replace[new_idx].clone();
                drop(snap);
                drop(state);
                ui.set_find_replace_text(text.into());
            });
        }

        // ── replace-history-next (↓ in replace input) ────────────────────────
        {
            let find_state = find_state.clone(); // clone required: captured by on_replace_history_next
            let history = history.clone(); // clone required: captured by on_replace_history_next
            let window_weak = window.as_weak();
            ui_state.on_replace_history_next(move || {
                let Some(w) = window_weak.upgrade() else {
                    return;
                };
                let ui = w.global::<crate::UiState>();
                let mut state = find_state.borrow_mut();
                match state.replace_hist_idx {
                    None => (),
                    Some(0) => {
                        state.replace_hist_idx = None;
                        let draft = state.replace_draft.clone();
                        drop(state);
                        ui.set_find_replace_text(draft.into());
                    }
                    Some(i) => {
                        let snap = history.lock().unwrap_or_else(|p| p.into_inner());
                        let new_idx = i - 1;
                        state.replace_hist_idx = Some(new_idx);
                        let text = snap.replace[new_idx].clone();
                        drop(snap);
                        drop(state);
                        ui.set_find_replace_text(text.into());
                    }
                }
            });
        }

        // ── commit-search: persist find query on Enter ────────────────────────
        {
            let find_state = find_state.clone(); // clone required: captured by on_commit_search
            let history = history.clone(); // clone required: captured by on_commit_search
            let svc = find_history_svc.clone(); // clone required: captured by on_commit_search
            let window_weak = window.as_weak();
            ui_state.on_commit_search(move || {
                let Some(w) = window_weak.upgrade() else {
                    return;
                };
                let ui = w.global::<crate::UiState>();
                let query = ui.get_find_query().to_string();
                if query.is_empty() {
                    return;
                }
                {
                    let mut state = find_state.borrow_mut();
                    state.find_hist_idx = None;
                    state.find_draft = String::new();
                }
                {
                    let mut snap = history.lock().unwrap_or_else(|p| p.into_inner());
                    if !snap.find.contains(&query) {
                        snap.find.insert(0, query.clone());
                        snap.find.truncate(30);
                    }
                }
                let svc = svc.clone(); // clone required: moved into tokio::spawn
                tokio::spawn(async move {
                    if let Err(e) = svc.save("find", &query).await {
                        tracing::warn!(error = %e, %query, "failed to save find history");
                    }
                });
            });
        }

        // ── commit-replace: persist replace text on Enter ─────────────────────
        {
            let find_state = find_state.clone(); // clone required: captured by on_commit_replace
            let history = history.clone(); // clone required: captured by on_commit_replace
            let svc = find_history_svc.clone(); // clone required: captured by on_commit_replace
            let window_weak = window.as_weak();
            ui_state.on_commit_replace(move || {
                let Some(w) = window_weak.upgrade() else {
                    return;
                };
                let ui = w.global::<crate::UiState>();
                let text = ui.get_find_replace_text().to_string();
                if text.is_empty() {
                    return;
                }
                {
                    let mut state = find_state.borrow_mut();
                    state.replace_hist_idx = None;
                    state.replace_draft = String::new();
                }
                {
                    let mut snap = history.lock().unwrap_or_else(|p| p.into_inner());
                    if !snap.replace.contains(&text) {
                        snap.replace.insert(0, text.clone());
                        snap.replace.truncate(30);
                    }
                }
                let svc = svc.clone(); // clone required: moved into tokio::spawn
                tokio::spawn(async move {
                    if let Err(e) = svc.save("replace", &text).await {
                        tracing::warn!(error = %e, %text, "failed to save replace history");
                    }
                });
            });
        }

        // ── replace-one ──────────────────────────────────────────────────────
        {
            let find_state = find_state.clone(); // clone required: captured by on_replace_one
            let window_weak = window.as_weak();
            ui_state.on_replace_one(move || {
                let Some(w) = window_weak.upgrade() else {
                    return;
                };
                let ui = w.global::<crate::UiState>();
                let text = ui.get_editor_text().to_string();
                let query = ui.get_find_query().to_string();
                let replace = ui.get_find_replace_text().to_string();
                let cs = ui.get_find_case_sensitive();
                let rx = ui.get_find_use_regex();

                let mut state = find_state.borrow_mut();
                state.update(&text, &query, cs, rx);
                if state.matches.is_empty() {
                    return;
                }
                let (start, end) = state.matches[state.current];
                let mut new_text = text;
                new_text.replace_range(start..end, &replace);
                ui.set_editor_text(new_text.into());

                let updated = ui.get_editor_text().to_string();
                state.last_query = String::new(); // invalidate cache
                state.update(&updated, &query, cs, rx);
                if state.matches.is_empty() {
                    ui.set_find_current_match(0);
                    ui.set_find_total_matches(0);
                    ui.set_find_sel_start(-1);
                    ui.set_find_sel_end(-1);
                    return;
                }
                state.current = state.current.min(state.matches.len() - 1);
                let (s, e) = state.matches[state.current];
                ui.set_find_current_match((state.current + 1) as i32);
                ui.set_find_total_matches(state.matches.len() as i32);
                ui.set_find_sel_start(s as i32);
                ui.set_find_sel_end(e as i32);
            });
        }

        // ── replace-all ──────────────────────────────────────────────────────
        {
            let window_weak = window.as_weak();
            ui_state.on_replace_all(move || {
                let Some(w) = window_weak.upgrade() else {
                    return;
                };
                let ui = w.global::<crate::UiState>();
                let text = ui.get_editor_text().to_string();
                let query = ui.get_find_query().to_string();
                let replace = ui.get_find_replace_text().to_string();
                let cs = ui.get_find_case_sensitive();
                let rx = ui.get_find_use_regex();

                let matches = compute_matches(&text, &query, cs, rx);
                if matches.is_empty() {
                    return;
                }
                // Replace from end to start to preserve byte offsets.
                let mut new_text = text;
                for (start, end) in matches.into_iter().rev() {
                    new_text.replace_range(start..end, &replace);
                }
                ui.set_editor_text(new_text.into());
                ui.set_find_current_match(0);
                ui.set_find_total_matches(0);
                ui.set_find_sel_start(-1);
                ui.set_find_sel_end(-1);
            });
        }
    }

    // ── Metadata search palette (Ctrl+P) ─────────────────────────────────────

    fn register_metadata_search_callbacks(
        window: &crate::AppWindow,
        sidebar_state: Arc<Mutex<SidebarUiState>>,
    ) {
        let ui = window.global::<crate::UiState>();

        // metadata-search-open: check active connection; open palette if connected.
        {
            let window_weak = window.as_weak(); // clone required: on_metadata_search_open closure
            let sidebar_state_open = Arc::clone(&sidebar_state); // clone required: captured by open closure
            ui.on_metadata_search_open(move || {
                let Some(w) = window_weak.upgrade() else {
                    return;
                };
                let ui = w.global::<crate::UiState>();
                if ui.get_active_connection_id().is_empty() {
                    ui.set_status_message(t!("error.no_active_connection").as_ref().into());
                    return;
                }
                ui.set_metadata_search_query("".into());
                ui.set_metadata_search_selected(0);
                // Show all tables/views immediately on open (empty query).
                let active_id = ui.get_active_connection_id().to_string();
                let sb = sidebar_state_open.lock().unwrap_or_else(|p| p.into_inner());
                let items = if let Some(meta) = sb.metadata.get(&active_id) {
                    search_metadata("", meta)
                } else {
                    vec![]
                };
                drop(sb);
                let slint_items = items_to_slint(items);
                ui.set_metadata_search_results(Rc::new(slint::VecModel::from(slint_items)).into());
                ui.set_show_metadata_search(true);
            });
        }

        // metadata-search: recompute results for the current query.
        {
            let window_weak = window.as_weak(); // clone required: on_metadata_search closure
            let sidebar_state = Arc::clone(&sidebar_state); // clone required: on_metadata_search closure
            ui.on_metadata_search(move || {
                let Some(w) = window_weak.upgrade() else {
                    return;
                };
                let ui = w.global::<crate::UiState>();
                let query = ui.get_metadata_search_query().to_string();
                let active_id = ui.get_active_connection_id().to_string();
                let sb = sidebar_state.lock().unwrap_or_else(|p| p.into_inner());
                let items = if let Some(meta) = sb.metadata.get(&active_id) {
                    search_metadata(&query, meta)
                } else {
                    vec![]
                };
                drop(sb);
                let slint_items = items_to_slint(items);
                ui.set_metadata_search_results(Rc::new(slint::VecModel::from(slint_items)).into());
                ui.set_metadata_search_selected(0);
            });
        }

        // metadata-search-select: dispatch action based on result kind.
        {
            let window_weak = window.as_weak(); // clone required: on_metadata_search_select closure
            ui.on_metadata_search_select(move |kind, label, table_name| {
                let Some(w) = window_weak.upgrade() else {
                    return;
                };
                let ui = w.global::<crate::UiState>();
                ui.set_show_metadata_search(false);
                ui.set_metadata_search_query("".into());
                match kind.as_str() {
                    "table" | "view" => {
                        ui.invoke_table_double_clicked(label);
                    }
                    "column" => {
                        // Copy just the column name (not "table.column") to clipboard.
                        let col_name = label
                            .as_str()
                            .split('.')
                            .next_back()
                            .unwrap_or(label.as_str())
                            .to_string();
                        if let Ok(mut clip) = arboard::Clipboard::new() {
                            let _ = clip.set_text(col_name);
                        }
                        // Open the parent table.
                        ui.invoke_table_double_clicked(table_name);
                    }
                    _ => {}
                }
            });
        }
    }

    // ── Status callbacks (TODO) ───────────────────────────────────────────────

    fn register_status_callbacks(_window: &crate::AppWindow, _state: SharedState) {
        // Status bar text is updated by spawn_event_handler via invoke_from_event_loop.
        // No additional setup needed here.
    }

    // ── Snippet callbacks ────────────────────────────────────────────────────

    fn register_snippet_callbacks(window: &crate::AppWindow, snippet_repo: Arc<SnippetRepository>) {
        let ui = window.global::<crate::UiState>();

        // open-snippet-save: extract SQL from selection or cursor line, pre-fill dialog.
        {
            let ww = window.as_weak();
            ui.on_open_snippet_save(move |anchor_pos, cursor_pos| {
                let Some(w) = ww.upgrade() else { return };
                let ui = w.global::<crate::UiState>();
                let editor_text = ui.get_editor_text().to_string();
                let a = (anchor_pos as usize).min(editor_text.len());
                let c = (cursor_pos as usize).min(editor_text.len());
                let (start, end) = (a.min(c), a.max(c));
                let sql = if start < end {
                    editor_text[start..end].to_string()
                } else {
                    // Extract the cursor's line from the full editor text.
                    let byte_pos = c;
                    let line_start = editor_text[..byte_pos]
                        .rfind('\n')
                        .map(|i| i + 1)
                        .unwrap_or(0);
                    let line_end = editor_text[byte_pos..]
                        .find('\n')
                        .map(|i| byte_pos + i)
                        .unwrap_or(editor_text.len());
                    editor_text[line_start..line_end].trim().to_string()
                };
                if sql.is_empty() {
                    return;
                }
                ui.set_snippet_save_sql(sql.into());
                ui.set_snippet_save_comment("".into());
                ui.set_show_snippet_save(true);
            });
        }

        // save-snippet: persist the new entry with a sequential "Query N" name, refresh.
        {
            let repo = Arc::clone(&snippet_repo);
            let ww = window.as_weak();
            ui.on_save_snippet(move |_name, comment, sql| {
                let Some(w) = ww.upgrade() else { return };
                let ui = w.global::<crate::UiState>();
                let conn_id_str = ui.get_active_connection_id().to_string();
                let conn_id = if conn_id_str.is_empty() {
                    None
                } else {
                    Some(conn_id_str)
                };
                let comment = comment.to_string();
                let sql = sql.to_string();
                let repo_c = Arc::clone(&repo);
                let ww_c = ww.clone();
                tokio::spawn(async move {
                    let n = repo_c.next_query_number().await.unwrap_or(1);
                    let name = format!("Query {n}");
                    let entry = wf_config::snippet::SnippetEntry {
                        id: uuid::Uuid::new_v4().to_string(),
                        name,
                        comment,
                        connection_id: None,
                        sql,
                        created_at: chrono::Utc::now().to_rfc3339(),
                        sort_order: 0,
                    };
                    if let Err(e) = repo_c.add(&entry).await {
                        tracing::warn!(error = %e, "failed to save snippet");
                        return;
                    }
                    do_refresh_snippets(&ww_c, &repo_c, conn_id.as_deref()).await;
                });
            });
        }

        // save-snippet-edit: persist updated comment + sql, close edit dialog, refresh.
        {
            let repo = Arc::clone(&snippet_repo);
            let ww = window.as_weak();
            ui.on_save_snippet_edit(move |id, comment, sql| {
                let Some(w) = ww.upgrade() else { return };
                let ui = w.global::<crate::UiState>();
                let conn_id_str = ui.get_active_connection_id().to_string();
                let conn_id = if conn_id_str.is_empty() {
                    None
                } else {
                    Some(conn_id_str)
                };
                let id = id.to_string();
                let comment = comment.to_string();
                let sql = sql.to_string();
                let repo_c = Arc::clone(&repo);
                let ww_c = ww.clone();
                tokio::spawn(async move {
                    if let Err(e) = repo_c.update(&id, &comment, &sql).await {
                        tracing::warn!(error = %e, "failed to update snippet");
                        return;
                    }
                    do_refresh_snippets(&ww_c, &repo_c, conn_id.as_deref()).await;
                });
            });
        }

        // delete-snippet-item: remove from DB, refresh list.
        {
            let repo = Arc::clone(&snippet_repo);
            let ww = window.as_weak();
            ui.on_delete_snippet_item(move |id| {
                let Some(w) = ww.upgrade() else { return };
                let ui = w.global::<crate::UiState>();
                let conn_id_str = ui.get_active_connection_id().to_string();
                let conn_id = if conn_id_str.is_empty() {
                    None
                } else {
                    Some(conn_id_str)
                };
                let id = id.to_string();
                let repo_c = Arc::clone(&repo);
                let ww_c = ww.clone();
                tokio::spawn(async move {
                    if let Err(e) = repo_c.delete(&id).await {
                        tracing::warn!(error = %e, "failed to delete snippet");
                        return;
                    }
                    do_refresh_snippets(&ww_c, &repo_c, conn_id.as_deref()).await;
                });
            });
        }

        // load-snippet-sql: insert SQL at editor cursor position.
        {
            let ww = window.as_weak();
            ui.on_load_snippet_sql(move |sql, cursor_pos| {
                with_ui(&ww, |ui| {
                    let current = ui.get_editor_text().to_string();
                    let pos = (cursor_pos as usize).min(current.len());
                    let new_text = format!("{}{}{}", &current[..pos], sql, &current[pos..]);
                    let new_cursor = (pos + sql.len()) as i32;
                    ui.set_editor_text(new_text.into());
                    ui.set_editor_cursor_target(new_cursor);
                });
            });
        }

        // execute-snippet-sql: run snippet SQL directly without touching the editor.
        {
            let ww = window.as_weak();
            ui.on_execute_snippet_sql(move |sql| {
                let Some(w) = ww.upgrade() else { return };
                let ui = w.global::<crate::UiState>();
                ui.invoke_run_query(sql);
            });
        }

        // save-snippet-bar-position: persist dragged position.
        {
            let repo = Arc::clone(&snippet_repo);
            ui.on_save_snippet_bar_position(move |x, y| {
                let repo_c = Arc::clone(&repo);
                tokio::spawn(async move {
                    if let Err(e) = repo_c.set_bar_position(x, y).await {
                        tracing::warn!(error = %e, "failed to save snippet bar position");
                    }
                });
            });
        }
    }
}

// ── Snippet helpers ──────────────────────────────────────────────────────────

fn snippet_to_slint(b: wf_config::snippet::SnippetEntry) -> crate::SnippetEntry {
    crate::SnippetEntry {
        id: b.id.into(),
        name: b.name.into(),
        comment: b.comment.into(),
        sql: b.sql.into(),
    }
}

async fn do_refresh_snippets(
    ww: &slint::Weak<crate::AppWindow>,
    repo: &SnippetRepository,
    connection_id: Option<&str>,
) {
    let items = repo.list(connection_id).await.unwrap_or_default();
    let slint_items: Vec<crate::SnippetEntry> = items.into_iter().map(snippet_to_slint).collect();
    let ww_c = ww.clone();
    let _ = slint::invoke_from_event_loop(move || {
        with_ui(&ww_c, move |ui| {
            ui.set_snippets(Rc::new(slint::VecModel::from(slint_items)).into());
        });
    });
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn db_type_label(dt: &DbType) -> &'static str {
    match dt {
        DbType::PostgreSQL => "PostgreSQL",
        DbType::MySQL => "MySQL",
        DbType::SQLite => "SQLite",
    }
}

fn db_type_label_config(dt: &DbTypeName) -> &'static str {
    match dt {
        DbTypeName::PostgreSQL => "PostgreSQL",
        DbTypeName::MySQL => "MySQL",
        DbTypeName::SQLite => "SQLite",
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

    // Preserve the existing id when editing so save_connection upserts correctly.
    let edit_id = ui.get_form_edit_id().to_string();
    let id = if edit_id.is_empty() {
        uuid::Uuid::new_v4().to_string()
    } else {
        edit_id
    };

    let conn = DbConnection {
        id,
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

// ── Connection string helpers ─────────────────────────────────────────────────

/// Percent-encode a string for use in a URL userinfo or path component.
/// Only unreserved characters (RFC 3986) are left as-is.
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => {
                out.push('%');
                out.push_str(&format!("{:02X}", b));
            }
        }
    }
    out
}

/// Percent-decode a URL component back to a plain string.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2]))
        {
            out.push((hi << 4) | lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Derive a connection URL from individual fields.
///
/// SQLite: returns the database path as-is.
/// PostgreSQL/MySQL: builds `scheme://[user[:pass]@][host[:port]][/database]`.
fn derive_conn_string(
    db_type: &DbType,
    host: &str,
    port: Option<u16>,
    user: &str,
    password: &str,
    database: &str,
) -> String {
    if matches!(db_type, DbType::SQLite) {
        return database.to_string();
    }
    let (scheme, default_port) = match db_type {
        DbType::PostgreSQL => ("postgres", 5432u16),
        DbType::MySQL => ("mysql", 3306u16),
        DbType::SQLite => unreachable!(),
    };
    let userinfo = match (user.is_empty(), password.is_empty()) {
        (true, _) => String::new(),
        (false, true) => format!("{}@", percent_encode(user)),
        (false, false) => format!("{}:{}@", percent_encode(user), percent_encode(password)),
    };
    let hostport = if host.is_empty() {
        String::new()
    } else {
        format!("{}:{}", host, port.unwrap_or(default_port))
    };
    let db_part = if database.is_empty() {
        String::new()
    } else {
        format!("/{}", database)
    };
    format!("{}://{}{}{}", scheme, userinfo, hostport, db_part)
}

/// Parse a URL-format connection string into individual fields.
///
/// Handles `postgres://`, `postgresql://`, and `mysql://` schemes.
/// SQLite: the entire string is treated as the database path.
///
/// Returns `(host, port, user, password, database)` or `None` if the string
/// cannot be recognised as a URL for the given db type.
fn parse_conn_string(
    s: &str,
    db_type: &DbType,
) -> Option<(String, Option<u16>, String, String, String)> {
    let s = s.trim();
    if matches!(db_type, DbType::SQLite) {
        return Some((
            String::new(),
            None,
            String::new(),
            String::new(),
            s.to_string(),
        ));
    }
    let rest = s
        .strip_prefix("postgres://")
        .or_else(|| s.strip_prefix("postgresql://"))
        .or_else(|| s.strip_prefix("mysql://"))?;
    // Strip query string / fragment
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    // Split userinfo from host
    let (userinfo, hostdb) = match rest.rfind('@') {
        Some(pos) => (&rest[..pos], &rest[pos + 1..]),
        None => ("", rest),
    };
    let (user, password) = match userinfo.find(':') {
        Some(pos) => (&userinfo[..pos], &userinfo[pos + 1..]),
        None => (userinfo, ""),
    };
    let (hostport, database) = match hostdb.find('/') {
        Some(pos) => (&hostdb[..pos], &hostdb[pos + 1..]),
        None => (hostdb, ""),
    };
    // Handle IPv6 brackets: [::1]:5432
    let (host, port) = if hostport.starts_with('[') {
        let bracket_end = hostport.find(']').unwrap_or(hostport.len());
        let host_raw = &hostport[1..bracket_end];
        let port = hostport[bracket_end + 1..]
            .strip_prefix(':')
            .and_then(|p| p.parse::<u16>().ok());
        (host_raw, port)
    } else {
        match hostport.rfind(':') {
            Some(pos) => {
                let port = hostport[pos + 1..].parse::<u16>().ok();
                (&hostport[..pos], port)
            }
            None => (hostport, None),
        }
    };
    Some((
        percent_decode(host),
        port,
        percent_decode(user),
        percent_decode(password),
        percent_decode(database),
    ))
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

// ---------------------------------------------------------------------------
// Metadata search
// ---------------------------------------------------------------------------

/// A single metadata search result (not Slint-specific).
#[derive(Debug, Clone, PartialEq)]
struct SearchResultItem {
    kind: &'static str, // "table" | "view" | "column"
    label: String,
    detail: String,
    table_name: String,
}

/// Search `meta` for `query` (case-insensitive).
///
/// Empty query returns all tables and views (no columns).
/// Non-empty query searches table names, view names, and column names.
/// Prefix matches are ranked before substring matches. Results are capped at 50.
fn search_metadata(query: &str, meta: &DbMetadata) -> Vec<SearchResultItem> {
    if query.is_empty() {
        let mut results: Vec<SearchResultItem> = meta
            .tables
            .iter()
            .map(|t| SearchResultItem {
                kind: "table",
                label: t.name.clone(),
                detail: String::new(),
                table_name: t.name.clone(),
            })
            .chain(meta.views.iter().map(|v| SearchResultItem {
                kind: "view",
                label: v.name.clone(),
                detail: String::new(),
                table_name: v.name.clone(),
            }))
            .collect();
        results.truncate(50);
        return results;
    }

    let q = query.to_lowercase();
    let mut prefix: Vec<SearchResultItem> = vec![];
    let mut contains: Vec<SearchResultItem> = vec![];

    let classify = |item: SearchResultItem,
                    name_lower: &str,
                    p: &mut Vec<SearchResultItem>,
                    c: &mut Vec<SearchResultItem>| {
        if name_lower.starts_with(q.as_str()) {
            p.push(item);
        } else if name_lower.contains(q.as_str()) {
            c.push(item);
        }
    };

    for table in &meta.tables {
        let name_lower = table.name.to_lowercase();
        classify(
            SearchResultItem {
                kind: "table",
                label: table.name.clone(),
                detail: String::new(),
                table_name: table.name.clone(),
            },
            &name_lower,
            &mut prefix,
            &mut contains,
        );
        for col in &table.columns {
            let col_label = format!("{}.{}", table.name, col.name);
            let col_label_lower = col_label.to_lowercase();
            classify(
                SearchResultItem {
                    kind: "column",
                    label: col_label,
                    detail: col.data_type.clone(),
                    table_name: table.name.clone(),
                },
                &col_label_lower,
                &mut prefix,
                &mut contains,
            );
        }
    }

    for view in &meta.views {
        let name_lower = view.name.to_lowercase();
        classify(
            SearchResultItem {
                kind: "view",
                label: view.name.clone(),
                detail: String::new(),
                table_name: view.name.clone(),
            },
            &name_lower,
            &mut prefix,
            &mut contains,
        );
    }

    prefix.extend(contains);
    prefix.truncate(50);
    prefix
}

/// Convert `SearchResultItem`s to the Slint `MetadataSearchResult` type.
fn items_to_slint(items: Vec<SearchResultItem>) -> Vec<crate::MetadataSearchResult> {
    items
        .into_iter()
        .map(|r| crate::MetadataSearchResult {
            kind: r.kind.into(),
            label: r.label.into(),
            detail: r.detail.into(),
            table_name: r.table_name.into(),
        })
        .collect()
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

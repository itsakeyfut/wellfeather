mod appearance;
mod completion;
mod connection;
mod event;
mod find_replace;
mod metadata_search;
mod palette;
mod query;
mod snippet;
mod tabs;
mod tabs_state;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use slint::ComponentHandle;
use tokio::sync::mpsc;
use wf_config::snippet::SnippetRepository;
use wf_db::models::{DbMetadata, TableInfo};
use wf_history::find_history::FindHistoryService;
use wf_history::session::SessionService;
use wf_query::analyzer::has_dangerous_dml;

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

/// Acquire the sidebar lock with poison recovery, run `f`, and release.
fn with_sidebar<R>(s: &Arc<Mutex<SidebarUiState>>, f: impl FnOnce(&SidebarUiState) -> R) -> R {
    f(&s.lock().unwrap_or_else(|p| p.into_inner()))
}

/// Acquire the sidebar lock mutably with poison recovery, run `f`, and release.
fn with_sidebar_mut<R>(
    s: &Arc<Mutex<SidebarUiState>>,
    f: impl FnOnce(&mut SidebarUiState) -> R,
) -> R {
    f(&mut s.lock().unwrap_or_else(|p| p.into_inner()))
}

/// Map a slice of `GroupConfig` to Slint `GroupEntry` values.
fn groups_to_slint(groups: &[GroupConfig]) -> Vec<crate::GroupEntry> {
    groups
        .iter()
        .map(|g| crate::GroupEntry {
            id: g.id.clone().into(),
            name: g.name.clone().into(),
            color: parse_hex_color(&g.color),
        })
        .collect()
}

/// Map a slice of `ConnectionConfig` entries to Slint `ConnectionEntry` values.
/// Pass the active connection id; connections whose id matches get `is_active: true`.
/// Pass `""` to mark all entries as inactive.
fn config_connections_to_entries(
    conns: &[wf_config::models::ConnectionConfig],
    active_id: &str,
) -> Vec<crate::ConnectionEntry> {
    conns
        .iter()
        .map(|c| crate::ConnectionEntry {
            is_active: c.id == active_id,
            db_type: connection::db_type_label_config(&c.db_type).into(),
            name: c.name.clone().into(),
            id: c.id.clone().into(),
        })
        .collect()
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
        self.matches = find_replace::compute_matches(text, query, case_sensitive, use_regex);
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

use wf_config::models::{ConnectionConfig, GroupConfig, Theme};

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
    read_only: HashMap<String, bool>,
    /// All saved connections (from ConnectionRepository). Used to build the
    /// sidebar and DB manager list even when no DB is running.
    config_connections: Vec<ConnectionConfig>,
    /// All saved groups. Ordered for stable sidebar rendering.
    groups: Vec<GroupConfig>,
}

/// Parse a CSS hex color string (e.g. "#89b4fa") into a Slint `Color`.
/// Falls back to a neutral gray on malformed input.
fn parse_hex_color(s: &str) -> slint::Color {
    let s = s.trim_start_matches('#');
    if s.len() == 6
        && let (Ok(r), Ok(g), Ok(b)) = (
            u8::from_str_radix(&s[0..2], 16),
            u8::from_str_radix(&s[2..4], 16),
            u8::from_str_radix(&s[4..6], 16),
        )
    {
        return slint::Color::from_rgb_u8(r, g, b);
    }
    slint::Color::from_rgb_u8(0x6c, 0x70, 0x86) // overlay0 gray
}

#[allow(clippy::too_many_arguments)]
fn push_conn_node(
    nodes: &mut Vec<crate::SidebarNode>,
    conn: &ConnectionConfig,
    active_id: &str,
    metadata: &HashMap<String, DbMetadata>,
    expanded: &HashSet<String>,
    read_only: &HashMap<String, bool>,
    parent_index: i32,
    visible: bool,
    group_color: Option<slint::Color>,
) {
    let conn_node_id = format!("conn:{}", conn.id);
    let is_conn_expanded = expanded.contains(&conn_node_id);
    let conn_color = conn
        .color
        .as_deref()
        .map(parse_hex_color)
        .or(group_color)
        .unwrap_or_default();
    let has_conn_color = conn.color.is_some() || group_color.is_some();
    let conn_idx = nodes.len() as i32;
    nodes.push(crate::SidebarNode {
        id: conn_node_id.clone().into(),
        raw_id: conn.id.clone().into(),
        label: conn.name.clone().into(),
        sub_label: connection::db_type_label_config(&conn.db_type).into(),
        level: 0,
        is_expanded: is_conn_expanded,
        is_active: conn.id == active_id,
        is_read_only: *read_only.get(&conn.id).unwrap_or(&false),
        node_kind: "connection".into(),
        parent_index,
        visible,
        stagger_delay: 0,
        color: conn_color,
        has_color: has_conn_color,
    });
    let parent_visible = visible && is_conn_expanded;
    let Some(meta) = metadata.get(&conn.id) else {
        return;
    };
    push_tableinfo_category(
        nodes,
        conn_idx,
        &conn.id,
        "Tables",
        &meta.tables,
        "table",
        expanded,
        parent_visible,
    );
    push_tableinfo_category(
        nodes,
        conn_idx,
        &conn.id,
        "Views",
        &meta.views,
        "view",
        expanded,
        parent_visible,
    );
    push_string_category(
        nodes,
        conn_idx,
        &conn.id,
        "Stored Procedures",
        &meta.stored_procs,
        "proc",
        expanded,
        parent_visible,
    );
    push_string_category(
        nodes,
        conn_idx,
        &conn.id,
        "Indexes",
        &meta.indexes,
        "index",
        expanded,
        parent_visible,
    );
}

fn build_sidebar_tree(
    groups: &[GroupConfig],
    config_conns: &[ConnectionConfig],
    active_id: &str,
    metadata: &HashMap<String, DbMetadata>,
    expanded: &HashSet<String>,
    read_only: &HashMap<String, bool>,
) -> Vec<crate::SidebarNode> {
    let mut nodes = vec![];

    // --- Group nodes first, then their member connections ---
    for group in groups {
        let group_node_id = format!("group:{}", group.id);
        let is_group_expanded = expanded.contains(&group_node_id);
        let group_color = parse_hex_color(&group.color);
        let group_idx = nodes.len() as i32;
        nodes.push(crate::SidebarNode {
            id: group_node_id.clone().into(),
            raw_id: group.id.clone().into(),
            label: group.name.clone().into(),
            sub_label: "".into(),
            level: 0,
            is_expanded: is_group_expanded,
            is_active: false,
            is_read_only: false,
            node_kind: "group".into(),
            parent_index: -1,
            visible: true,
            stagger_delay: 0,
            color: group_color,
            has_color: true,
        });
        for conn in config_conns
            .iter()
            .filter(|c| c.group_id.as_deref() == Some(group.id.as_str()))
        {
            push_conn_node(
                &mut nodes,
                conn,
                active_id,
                metadata,
                expanded,
                read_only,
                group_idx,
                is_group_expanded,
                Some(group_color),
            );
        }
    }

    // --- Ungrouped connections ---
    for conn in config_conns.iter().filter(|c| c.group_id.is_none()) {
        push_conn_node(
            &mut nodes, conn, active_id, metadata, expanded, read_only, -1, true, None,
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
        raw_id: "".into(),
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
        color: Default::default(),
        has_color: false,
    });
    // Always emit children; visible flag drives Slint height/opacity animation.
    for (child_idx, item) in items.iter().enumerate() {
        nodes.push(crate::SidebarNode {
            id: format!("item:{}:{}:{}", conn_id, kind, item.name).into(),
            raw_id: "".into(),
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
            color: Default::default(),
            has_color: false,
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
        raw_id: "".into(),
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
        color: Default::default(),
        has_color: false,
    });
    // Always emit children; visible flag drives Slint height/opacity animation.
    for (child_idx, item) in items.iter().enumerate() {
        nodes.push(crate::SidebarNode {
            id: format!("item:{}:{}:{}", conn_id, kind, item).into(),
            raw_id: "".into(),
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
            color: Default::default(),
            has_color: false,
        });
    }
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
        initial_groups: Vec<GroupConfig>,
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
            groups: initial_groups,
            ..Default::default()
        }));

        // Shared storage for the unfiltered query result; written by the event
        // handler on QueryFinished, read by the filter callbacks on the UI thread.
        let original_data: SharedOriginalData = Arc::new(Mutex::new(None));

        // Shared slot for the SSH fingerprint approval oneshot sender.
        // Written by the async event handler when a new fingerprint needs user approval;
        // consumed by the approve/reject UI callbacks on the Slint thread.
        let fp_approval_tx: Arc<Mutex<Option<tokio::sync::oneshot::Sender<bool>>>> =
            Arc::new(Mutex::new(None));

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

        connection::register_sidebar_callbacks(
            &window,
            state.clone(),
            tx_cmd.clone(),
            Arc::clone(&sidebar_state),
            enc_key,
            Rc::clone(&tabs_state),
        );
        connection::register_connection_form_callbacks(
            &window,
            tx_cmd.clone(),
            enc_key,
            Arc::clone(&fp_approval_tx),
        );
        query::register_editor_callbacks(&window, tx_cmd.clone());
        completion::register_completion_callbacks(&window, tx_cmd.clone());
        completion::register_completion_accept_callback(&window);
        let hl_model: Rc<slint::VecModel<crate::HighlightSpan>> =
            Rc::new(slint::VecModel::from(vec![]));
        window
            .global::<crate::UiState>()
            .set_highlight_spans(hl_model.clone().into());
        query::register_formatter_callback(&window, hl_model.clone());
        query::register_export_callbacks(&window, Arc::clone(&original_data), state.clone());
        appearance::register_theme_callback(&window, state.clone(), tx_cmd.clone());
        appearance::register_reduce_motion_callback(&window, tx_cmd.clone());
        appearance::register_menu_callbacks(&window, tx_cmd.clone());
        appearance::register_close_handler(&window, Rc::clone(&tabs_state), session_svc.clone());
        tabs::register_tab_callbacks(
            &window,
            tx_cmd.clone(),
            Rc::clone(&tabs_state),
            Arc::clone(&sidebar_state),
            hl_model.clone(),
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
        ui_global.set_tab_width(config.editor.tab_width as i32);
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
            let slint_tabs = tabs::tabs_to_slint(&ts.tabs);
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
        // Populate the connection list, group list, and sidebar at startup.
        let (startup_entries, startup_groups, startup_nodes) = with_sidebar(&sidebar_state, |sb| {
            let entries = config_connections_to_entries(&sb.config_connections, "");
            let group_entries = groups_to_slint(&sb.groups);
            let nodes = build_sidebar_tree(
                &sb.groups,
                &sb.config_connections,
                "",
                &sb.metadata,
                &sb.expanded,
                &sb.read_only,
            );
            (entries, group_entries, nodes)
        });
        ui_global.set_connection_list(Rc::new(slint::VecModel::from(startup_entries)).into());
        ui_global.set_group_list(Rc::new(slint::VecModel::from(startup_groups)).into());
        ui_global.set_sidebar_tree(Rc::new(slint::VecModel::from(startup_nodes)).into());

        query::register_result_callbacks(
            &window,
            state.clone(),
            Arc::clone(&original_data),
            tx_cmd.clone(),
        );
        appearance::register_language_callback(&window, tx_cmd.clone());
        find_replace::register_find_replace_callbacks(&window, find_history_svc);
        snippet::register_snippet_callbacks(&window, Arc::clone(&snippet_repo));
        metadata_search::register_metadata_search_callbacks(&window, Arc::clone(&sidebar_state));
        palette::register_command_palette_callbacks(
            &window,
            Arc::clone(&sidebar_state),
            tx_cmd.clone(),
            enc_key,
        );
        appearance::register_editor_prefs_callbacks(&window, tx_cmd.clone());
        appearance::register_highlight_callbacks(&window, hl_model.clone());

        // Highlight the editor text that was already set from session / tab restore.
        {
            let initial_text = ui_global.get_editor_text().to_string();
            if !initial_text.is_empty() {
                let spans = appearance::compute_highlight_spans(&initial_text);
                appearance::apply_highlight_spans(&hl_model, spans);
            }
        }

        // Load initial snippets (global only — no connection yet).
        {
            let initial_bk = handle.block_on(snippet_repo.list(None)).unwrap_or_default();
            let slint_bk: Vec<crate::SnippetEntry> = initial_bk
                .into_iter()
                .map(snippet::snippet_to_slint)
                .collect();
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

        event::spawn_event_handler(
            &window,
            rx_event,
            state,
            Arc::clone(&sidebar_state),
            Arc::clone(&original_data),
            Arc::clone(&snippet_repo),
            Arc::clone(&fp_approval_tx),
        );

        Ok(Self { window })
    }

    pub fn run(&self) -> Result<()> {
        self.window.run()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wf_config::models::{ConnectionConfig, DbTypeName, GroupConfig};

    fn make_group(id: &str, name: &str) -> GroupConfig {
        GroupConfig {
            id: id.to_string(),
            name: name.to_string(),
            color: "#6c7086".to_string(),
            expanded: true,
        }
    }

    fn make_conn_with_group(id: &str, name: &str, group_id: Option<&str>) -> ConnectionConfig {
        ConnectionConfig {
            group_id: group_id.map(|s| s.to_string()),
            ..make_conn(id, name)
        }
    }

    fn make_conn(id: &str, name: &str) -> ConnectionConfig {
        ConnectionConfig {
            id: id.to_string(),
            name: name.to_string(),
            db_type: DbTypeName::SQLite,
            connection_string: None,
            host: None,
            port: None,
            user: None,
            password_encrypted: None,
            database: None,
            safe_dml: true,
            read_only: false,
            ssh_enabled: false,
            ssh_host: None,
            ssh_port: None,
            ssh_user: None,
            ssh_auth_method: wf_config::models::SshAuthMethod::Password,
            ssh_password_encrypted: None,
            ssh_key_path: None,
            ssh_passphrase_encrypted: None,
            ssl_enabled: false,
            ssl_mode: wf_config::models::SslMode::Require,
            ssl_ca_cert: None,
            ssl_client_cert: None,
            ssl_client_key: None,
            group_id: None,
            color: None,
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

    // ── build_sidebar_tree ────────────────────────────────────────────────────

    #[test]
    fn build_sidebar_tree_should_render_connection_nodes() {
        let conns = vec![make_conn("a", "Alpha"), make_conn("b", "Beta")];
        let nodes = build_sidebar_tree(
            &[],
            &conns,
            "",
            &HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );
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
        let nodes = build_sidebar_tree(&[], &conns, "a", &metadata, &expanded, &HashMap::new());
        // conn + Tables + users(visible=false) + Views + Stored Procedures + Indexes = 6 nodes
        // Children are always emitted; visible flag drives animation.
        assert_eq!(nodes.len(), 6);
        assert_eq!(nodes[1].label.as_str(), "Tables");
        assert_eq!(nodes[1].level, 1);
        assert_eq!(nodes[1].node_kind.as_str(), "category");
        // "users" is emitted but invisible (Tables category not expanded)
        assert_eq!(nodes[2].label.as_str(), "users");
        assert!(!nodes[2].visible);
    }

    #[test]
    fn build_sidebar_tree_should_show_items_when_category_expanded() {
        let conns = vec![make_conn("a", "Alpha")];
        let mut expanded = HashSet::new();
        expanded.insert("conn:a".to_string());
        expanded.insert("cat:a:Tables".to_string());
        let mut metadata = HashMap::new();
        metadata.insert("a".to_string(), make_meta(&["users", "orders"]));
        let nodes = build_sidebar_tree(&[], &conns, "a", &metadata, &expanded, &HashMap::new());
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
        let nodes = build_sidebar_tree(
            &[],
            &conns,
            "a",
            &HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );
        // No metadata → no child nodes emitted at all.
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].level, 0);
    }

    #[test]
    fn build_sidebar_tree_should_emit_invisible_children_for_animation() {
        let conns = vec![make_conn("a", "Alpha")];
        let mut metadata = HashMap::new();
        metadata.insert("a".to_string(), make_meta(&["users"]));
        // Connection collapsed (not in expanded)
        let nodes = build_sidebar_tree(
            &[],
            &conns,
            "a",
            &metadata,
            &HashSet::new(),
            &HashMap::new(),
        );
        // Categories are emitted but invisible
        assert!(nodes.len() > 1);
        for node in nodes.iter().skip(1) {
            assert!(
                !node.visible,
                "category should be invisible when conn collapsed"
            );
        }
    }

    #[test]
    fn build_sidebar_tree_should_mark_active_connection() {
        let conns = vec![make_conn("a", "Alpha"), make_conn("b", "Beta")];
        let nodes = build_sidebar_tree(
            &[],
            &conns,
            "b",
            &HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );
        assert!(!nodes[0].is_active);
        assert!(nodes[1].is_active);
    }

    #[test]
    fn build_sidebar_tree_should_render_group_with_member_connection() {
        let groups = vec![make_group("g1", "Production")];
        let conns = vec![make_conn_with_group("c1", "Alpha", Some("g1"))];
        let mut expanded = HashSet::new();
        expanded.insert("group:g1".to_string());
        let nodes = build_sidebar_tree(
            &groups,
            &conns,
            "",
            &HashMap::new(),
            &expanded,
            &HashMap::new(),
        );
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].node_kind.as_str(), "group");
        assert_eq!(nodes[0].label.as_str(), "Production");
        assert_eq!(nodes[1].node_kind.as_str(), "connection");
        assert_eq!(nodes[1].parent_index, 0);
    }

    #[test]
    fn build_sidebar_tree_should_place_ungrouped_connections_after_groups() {
        let groups = vec![make_group("g1", "Prod")];
        let conns = vec![
            make_conn_with_group("c1", "Ungrouped", None),
            make_conn_with_group("c2", "InGroup", Some("g1")),
        ];
        let nodes = build_sidebar_tree(
            &groups,
            &conns,
            "",
            &HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );
        // group node → grouped conn → ungrouped conn
        assert_eq!(nodes[0].node_kind.as_str(), "group");
        assert_eq!(nodes[1].label.as_str(), "InGroup");
        assert_eq!(nodes[2].label.as_str(), "Ungrouped");
        assert_eq!(nodes[2].parent_index, -1);
    }

    // ── parse_hex_color ───────────────────────────────────────────────────────

    #[test]
    fn parse_hex_color_should_parse_valid_hex() {
        let c = parse_hex_color("#e74c3c");
        assert_eq!(c, slint::Color::from_rgb_u8(0xe7, 0x4c, 0x3c));
    }

    #[test]
    fn parse_hex_color_should_parse_hex_without_leading_hash() {
        let c = parse_hex_color("89b4fa");
        assert_eq!(c, slint::Color::from_rgb_u8(0x89, 0xb4, 0xfa));
    }

    #[test]
    fn parse_hex_color_should_fall_back_on_malformed_input() {
        let c = parse_hex_color("not-a-color");
        assert_eq!(c, slint::Color::from_rgb_u8(0x6c, 0x70, 0x86));
    }
}

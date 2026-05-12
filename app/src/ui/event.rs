use std::rc::Rc;
use std::sync::{Arc, Mutex};

use rust_i18n::t;
use slint::ComponentHandle;
use tokio::sync::mpsc;
use wf_config::models::{ConnectionConfig, Theme};
use wf_config::snippet::SnippetRepository;
use wf_db::models::DbMetadata;

use crate::app::event::{Event, StateEvent};
use crate::state::SharedState;

use super::completion::handle_completion_ready;
use super::query::{handle_query_finished, handle_table_data_loaded};
use super::snippet::do_refresh_snippets;
use super::{
    ERROR_TRUNCATION_CHARS, SharedOriginalData, SidebarUiState, build_sidebar_tree,
    config_connections_to_entries, with_sidebar, with_sidebar_mut, with_ui,
};

pub(super) fn spawn_event_handler(
    window: &crate::AppWindow,
    mut rx_event: mpsc::Receiver<Event>,
    state: SharedState,
    sidebar_state: Arc<Mutex<SidebarUiState>>,
    original_data: SharedOriginalData,
    snippet_repo: Arc<SnippetRepository>,
    fp_approval_tx: Arc<Mutex<Option<tokio::sync::oneshot::Sender<bool>>>>,
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
                    let ssh_active = connections
                        .iter()
                        .find(|c| c.id == id)
                        .map(|c| c.ssh_enabled)
                        .unwrap_or(false);
                    handle_connected(
                        id,
                        connections,
                        safe_dml,
                        read_only,
                        ssh_active,
                        window_weak.clone(),
                        Arc::clone(&sidebar_state),
                    );
                    // Refresh snippets to include per-connection entries.
                    let bk_repo = Arc::clone(&snippet_repo);
                    let bk_ww = window_weak.clone();
                    tokio::spawn(async move {
                        do_refresh_snippets(&bk_ww, &bk_repo, Some(&conn_id)).await;
                    });
                }
                Event::TestConnectionOk => handle_test_ok(window_weak.clone()),
                Event::TestConnectionFailed(msg) => handle_test_failed(msg, window_weak.clone()),
                Event::ConnectError(msg) => handle_connect_error(msg, window_weak.clone()),
                Event::QueryStarted => handle_query_started(window_weak.clone()),
                Event::QueryFinished(result) => {
                    handle_query_finished(result, window_weak.clone(), Arc::clone(&original_data))
                }
                Event::QueryCancelled => handle_query_cancelled(window_weak.clone()),
                Event::QueryError(msg) => handle_query_error(msg, window_weak.clone()),
                Event::Disconnected(id) => {
                    handle_disconnected(id, window_weak.clone());
                    // Drop per-connection snippets; show global only.
                    let bk_repo = Arc::clone(&snippet_repo);
                    let bk_ww = window_weak.clone();
                    tokio::spawn(async move {
                        do_refresh_snippets(&bk_ww, &bk_repo, None).await;
                    });
                }
                Event::ConnectionRemoved(id) => {
                    handle_connection_removed(id, window_weak.clone(), Arc::clone(&sidebar_state))
                }
                Event::MetadataLoaded(conn_id, meta) => handle_metadata_loaded(
                    conn_id,
                    meta,
                    window_weak.clone(),
                    state.clone(),
                    Arc::clone(&sidebar_state),
                ),
                Event::MetadataFetchFailed(msg) => {
                    handle_metadata_fetch_failed(msg, window_weak.clone())
                }
                Event::CompletionReady(items) => {
                    handle_completion_ready(items, window_weak.clone())
                }
                Event::StateChanged(StateEvent::ThemeChanged(t)) => {
                    handle_theme_changed(t, window_weak.clone())
                }
                Event::DdlLoaded { tab_id, ddl } => {
                    handle_ddl_loaded(tab_id, ddl, window_weak.clone())
                }
                Event::DdlFetchFailed { tab_id, msg } => {
                    handle_ddl_fetch_failed(tab_id, msg, window_weak.clone())
                }
                Event::TableDataLoaded { tab_id, result } => {
                    handle_table_data_loaded(tab_id, result, window_weak.clone())
                }
                Event::TableDataFailed { tab_id, msg } => {
                    handle_table_data_failed(tab_id, msg, window_weak.clone())
                }
                Event::ConnectionFlagsUpdated {
                    id,
                    safe_dml,
                    read_only,
                } => handle_connection_flags_updated(
                    id,
                    safe_dml,
                    read_only,
                    window_weak.clone(),
                    state.clone(),
                    Arc::clone(&sidebar_state),
                ),
                Event::SshFingerprintRequired {
                    fingerprint,
                    approval_tx,
                } => {
                    // Store the sender so the approve/reject callbacks can consume it.
                    *fp_approval_tx.lock().unwrap_or_else(|p| p.into_inner()) = Some(approval_tx);
                    let ww = window_weak.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        with_ui(&ww, move |ui| {
                            ui.set_ssh_fingerprint_text(fingerprint.into());
                            ui.set_show_ssh_fingerprint_dialog(true);
                        });
                    });
                }
                _ => {}
            }
        }
    });
}

// ── Per-event handlers ─────────────────────────────────────────────────────────

fn handle_connected(
    id: String,
    connections: Vec<ConnectionConfig>,
    safe_dml: bool,
    read_only: bool,
    ssh_active: bool,
    ww: slint::Weak<crate::AppWindow>,
    sidebar_state: Arc<Mutex<SidebarUiState>>,
) {
    // Build Send data outside invoke_from_event_loop (Rc<VecModel> is not Send).
    let entries = config_connections_to_entries(&connections, &id);
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
    with_sidebar_mut(&sidebar_state, |sb| {
        sb.expanded.insert(format!("conn:{}", id));
        sb.config_connections = connections.clone();
        sb.read_only = connections
            .iter()
            .map(|c| (c.id.clone(), c.read_only))
            .collect();
    });
    let sidebar_nodes = with_sidebar(&sidebar_state, |sb| {
        build_sidebar_tree(
            &sb.config_connections,
            &id,
            &sb.metadata,
            &sb.expanded,
            &sb.read_only,
        )
    });
    // clone required: invoke_from_event_loop closure must be 'static
    let _ = slint::invoke_from_event_loop(move || {
        with_ui(&ww, move |ui| {
            let model = Rc::new(slint::VecModel::from(entries));
            ui.set_connection_list(model.into());
            ui.set_active_connection_id(id.into());
            ui.set_conn_safe_dml(safe_dml);
            ui.set_conn_read_only(read_only);
            ui.set_ssh_active(ssh_active);
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
    with_sidebar_mut(&sidebar_state, |sb| {
        sb.read_only.insert(id.clone(), read_only);
        if let Some(cc) = sb.config_connections.iter_mut().find(|c| c.id == id) {
            cc.safe_dml = safe_dml;
            cc.read_only = read_only;
        }
    });
    let active_id = state.conn.active().map(|c| c.id).unwrap_or_default();
    let sidebar_nodes = with_sidebar(&sidebar_state, |sb| {
        build_sidebar_tree(
            &sb.config_connections,
            &active_id,
            &sb.metadata,
            &sb.expanded,
            &sb.read_only,
        )
    });
    // Recompute the status bar label only when this is the active connection.
    let active_id = state
        .conn
        .active()
        .map(|c| c.id.clone())
        .unwrap_or_default();
    let is_active = active_id == id;
    let new_status = if is_active {
        let base = {
            with_sidebar(&sidebar_state, |sb| {
                sb.config_connections
                    .iter()
                    .find(|c| c.id == id)
                    .map(|c| match c.database.as_deref() {
                        Some(db) if !db.is_empty() => format!("{} / {}", c.name, db),
                        _ => c.name.clone(),
                    })
                    .unwrap_or_else(|| id.clone())
            })
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
            ui.set_ssh_active(false);
        });
    });
}

fn handle_connection_removed(
    id: String,
    ww: slint::Weak<crate::AppWindow>,
    sidebar_state: Arc<Mutex<SidebarUiState>>,
) {
    with_sidebar_mut(&sidebar_state, |sb| {
        sb.config_connections.retain(|c| c.id != id);
        sb.read_only.remove(&id);
        sb.metadata.remove(&id);
        sb.expanded.remove(&format!("conn:{}", id));
    });
    let (entries, sidebar_nodes) = with_sidebar(&sidebar_state, |sb| {
        let e = config_connections_to_entries(&sb.config_connections, "");
        let nodes = build_sidebar_tree(
            &sb.config_connections,
            "",
            &sb.metadata,
            &sb.expanded,
            &sb.read_only,
        );
        (e, nodes)
    });
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
    with_sidebar_mut(&sidebar_state, |sb| {
        sb.metadata.insert(conn_id, meta);
    });
    let active_id = state.conn.active().map(|c| c.id).unwrap_or_default();
    let nodes = with_sidebar(&sidebar_state, |sb| {
        build_sidebar_tree(
            &sb.config_connections,
            &active_id,
            &sb.metadata,
            &sb.expanded,
            &sb.read_only,
        )
    });
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

fn handle_table_data_failed(_tab_id: String, msg: String, ww: slint::Weak<crate::AppWindow>) {
    // clone required: invoke_from_event_loop closure must be 'static
    let _ = slint::invoke_from_event_loop(move || {
        with_ui(&ww, move |ui| {
            ui.set_tv_data_loading(false);
            ui.set_tv_data_error(msg.into());
        });
    });
}

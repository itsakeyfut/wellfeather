use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use slint::ComponentHandle;
use tokio::sync::mpsc;

use crate::app::command::Command;

use super::appearance::{apply_highlight_spans, compute_highlight_spans};
use super::undo::TextUndoState;
use super::{SidebarUiState, send_cmd, tabs_state, with_sidebar, with_ui};

pub(super) fn tabs_to_slint(tabs: &[tabs_state::TabEntry]) -> Vec<crate::TabEntry> {
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

pub(super) fn columns_to_slint(cols: &[wf_db::models::ColumnInfo]) -> Vec<crate::ColumnData> {
    cols.iter()
        .map(|c| crate::ColumnData {
            name: c.name.clone().into(),
            data_type: c.data_type.clone().into(),
            nullable: c.nullable,
        })
        .collect()
}

pub(super) fn register_tab_callbacks(
    window: &crate::AppWindow,
    tx_cmd: mpsc::Sender<Command>,
    tabs_state: Rc<RefCell<tabs_state::TabsState>>,
    sidebar_state: Arc<Mutex<SidebarUiState>>,
    hl_model: Rc<slint::VecModel<crate::HighlightSpan>>,
    undo_state: Rc<TextUndoState>,
) {
    let ui = window.global::<crate::UiState>();

    // on_new_tab: save current editor text, add a SQL Editor tab, switch to it.
    {
        let window_weak = window.as_weak();
        let tabs_state = Rc::clone(&tabs_state); // clone required: callback closure needs owned tabs_state
        let undo_state = Rc::clone(&undo_state); // clone required: on_new_tab closure
        ui.on_new_tab(move || {
            let current_text = window_weak
                .upgrade()
                .map(|w| w.global::<crate::UiState>().get_editor_text().to_string())
                .unwrap_or_default();
            let current_sub_tab = window_weak
                .upgrade()
                .map(|w| w.global::<crate::UiState>().get_tv_sub_tab() as usize)
                .unwrap_or(0);
            let (slint_tabs, active_idx, new_tab_id) = {
                let mut ts = tabs_state.borrow_mut();
                ts.save_current_text(&current_text);
                ts.save_tv_sub_tab(current_sub_tab);
                let (id, _) = ts.add_sql_editor();
                let slint_tabs = tabs_to_slint(&ts.tabs);
                let active_idx = ts.active_index as i32;
                (slint_tabs, active_idx, id)
            };
            // Reset undo debounce state when switching to a fresh tab.
            *undo_state.debounce.borrow_mut() = None;
            *undo_state.burst_start.borrow_mut() = None;
            *undo_state.last_known.borrow_mut() = String::new();
            with_ui(&window_weak, |ui| {
                ui.set_tabs(Rc::new(slint::VecModel::from(slint_tabs)).into());
                ui.set_active_tab_index(active_idx);
                ui.set_active_tab_kind_sql(true);
                ui.set_editor_text("".into());
                ui.set_editor_active_tab_id(new_tab_id.into());
            });
        });
    }

    // on_switch_tab: save current text, switch active tab, restore tab content.
    {
        let window_weak = window.as_weak();
        let tabs_state = Rc::clone(&tabs_state); // clone required: callback closure needs owned tabs_state
        let sidebar_state = Arc::clone(&sidebar_state); // clone required: callback closure needs owned sidebar_state
        let hl_model = Rc::clone(&hl_model); // clone required: on_switch_tab closure
        let undo_state = Rc::clone(&undo_state); // clone required: on_switch_tab closure
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
            let (
                slint_tabs,
                active_idx,
                tab_id,
                kind_sql,
                editor_text,
                tv_name,
                tv_cols,
                tv_sub_tab,
            ) = {
                let mut ts = tabs_state.borrow_mut();
                ts.save_current_text(&current_text);
                ts.save_tv_sub_tab(current_sub_tab);
                ts.set_active(i);
                let slint_tabs = tabs_to_slint(&ts.tabs);
                let active_idx = ts.active_index as i32;
                let tab_id = ts.active_tab().map(|t| t.id.clone()).unwrap_or_default();
                match ts.active_tab().map(|t| t.kind.clone()) {
                    Some(tabs_state::TabKind::SqlEditor { query_text }) => (
                        slint_tabs,
                        active_idx,
                        tab_id,
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
                        let cols = with_sidebar(&sidebar_state, |sb| {
                            sb.metadata
                                .get(&conn_id)
                                .and_then(|meta| {
                                    meta.tables
                                        .iter()
                                        .chain(meta.views.iter())
                                        .find(|t| t.name == table_name)
                                        .map(|ti| columns_to_slint(&ti.columns))
                                })
                                .unwrap_or_default()
                        });
                        (
                            slint_tabs,
                            active_idx,
                            tab_id,
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
                        tab_id,
                        true,
                        String::new(),
                        String::new(),
                        vec![],
                        0usize,
                    ),
                }
            };
            // Reset undo debounce when switching tabs — next keystroke's burst_start
            // must capture the new tab's text, not the old tab's.
            *undo_state.debounce.borrow_mut() = None;
            *undo_state.burst_start.borrow_mut() = None;
            *undo_state.last_known.borrow_mut() = editor_text.clone();
            let spans = if kind_sql {
                compute_highlight_spans(&editor_text)
            } else {
                vec![]
            };
            with_ui(&window_weak, |ui| {
                ui.set_tabs(Rc::new(slint::VecModel::from(slint_tabs)).into());
                ui.set_active_tab_index(active_idx);
                ui.set_active_tab_kind_sql(kind_sql);
                ui.set_editor_active_tab_id(tab_id.into());
                if kind_sql {
                    ui.set_editor_text(editor_text.into());
                    apply_highlight_spans(&hl_model, spans);
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
        let hl_model = Rc::clone(&hl_model); // clone required: on_close_tab closure
        let undo_state = Rc::clone(&undo_state); // clone required: on_close_tab closure
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
            let (tab_id, kind_sql, editor_text) = match ts
                .active_tab()
                .map(|t| (t.id.clone(), t.kind.clone()))
            {
                Some((id, tabs_state::TabKind::SqlEditor { query_text })) => (id, true, query_text),
                Some((id, _)) => (id, false, String::new()),
                None => (String::new(), true, String::new()),
            };
            drop(ts);
            // Reset undo debounce for the newly active tab.
            *undo_state.debounce.borrow_mut() = None;
            *undo_state.burst_start.borrow_mut() = None;
            *undo_state.last_known.borrow_mut() = editor_text.clone();
            let spans = if kind_sql {
                compute_highlight_spans(&editor_text)
            } else {
                vec![]
            };
            with_ui(&window_weak, |ui| {
                ui.set_tabs(Rc::new(slint::VecModel::from(slint_tabs)).into());
                ui.set_active_tab_index(active_idx);
                ui.set_active_tab_kind_sql(kind_sql);
                ui.set_editor_active_tab_id(tab_id.into());
                if kind_sql {
                    ui.set_editor_text(editor_text.into());
                    apply_highlight_spans(&hl_model, spans);
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
            let kind = with_sidebar(&sidebar_state, |sb| {
                if let Some(meta) = sb.metadata.get(&conn_id) {
                    if meta.views.iter().any(|v| v.name == tv_table_name) {
                        "view".to_string()
                    } else {
                        "table".to_string()
                    }
                } else {
                    "table".to_string()
                }
            });
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

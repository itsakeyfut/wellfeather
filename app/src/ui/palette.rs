use std::rc::Rc;
use std::sync::{Arc, Mutex};

use slint::{ComponentHandle, Model as _};
use tokio::sync::mpsc;

use crate::app::session::config_to_db_conn;
use crate::app::{command::Command, command_registry};

use super::{SidebarUiState, send_cmd, with_sidebar};

/// Collect the current saved connections as `(id, name)` pairs without
/// holding the sidebar lock across any other work.
fn current_connections(sidebar_state: &Arc<Mutex<SidebarUiState>>) -> Vec<(String, String)> {
    sidebar_state
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .config_connections
        .iter()
        .map(|c| (c.id.clone(), c.name.clone()))
        .collect()
}

/// Convert [`command_registry::PaletteAction`] values to Slint
/// `CommandPaletteItem` structs for display in the palette.
fn palette_items_to_slint(
    items: &[command_registry::PaletteAction],
) -> Vec<crate::CommandPaletteItem> {
    items
        .iter()
        .map(|a| crate::CommandPaletteItem {
            id: a.id.as_str().into(),
            label: a.label.as_str().into(),
            shortcut: a.shortcut.into(),
        })
        .collect()
}

/// Dispatch a command palette selection to the appropriate `UiState` action.
///
/// Called on the Slint event-loop thread so `ui.invoke_*` and property setters
/// are safe to call directly.
fn dispatch_palette_command(ui: &crate::UiState, id: &str) {
    match id {
        "run-all" => {
            let sql = ui.get_editor_text().to_string();
            ui.invoke_run_all(sql.into());
        }
        "cancel-query" => {
            ui.invoke_cancel_query();
        }
        "format-sql" => {
            ui.invoke_format_sql();
        }
        "find" if ui.get_active_tab_kind_sql() => {
            ui.set_find_bar_with_replace(false);
            ui.set_show_find_bar(true);
        }
        "find-replace" if ui.get_active_tab_kind_sql() => {
            ui.set_find_bar_with_replace(true);
            ui.set_show_find_bar(true);
        }
        "new-tab" => {
            ui.invoke_new_tab();
        }
        "close-tab" => {
            ui.invoke_close_tab(ui.get_active_tab_index());
        }
        "next-tab" => {
            let n = ui.get_tabs().row_count() as i32;
            if n > 0 {
                ui.invoke_switch_tab((ui.get_active_tab_index() + 1) % n);
            }
        }
        "prev-tab" => {
            let n = ui.get_tabs().row_count() as i32;
            if n > 0 {
                ui.invoke_switch_tab((ui.get_active_tab_index() + n - 1) % n);
            }
        }
        "toggle-snippet-bar" => {
            ui.set_show_snippet_bar(!ui.get_show_snippet_bar());
        }
        "open-metadata-search" => {
            ui.invoke_metadata_search_open();
        }
        "open-snippet-palette" => {
            ui.invoke_open_snippet_palette();
        }
        "save-snippet" => {
            ui.invoke_open_snippet_save(0, 0);
        }
        "show-snippets" => {
            ui.set_show_snippet_list(true);
        }
        "export-csv" => {
            ui.invoke_export_csv();
        }
        "export-json" => {
            ui.invoke_export_json();
        }
        "export-insert-sql" => {
            ui.invoke_export_insert_sql();
        }
        "open-db-manager" => {
            ui.invoke_open_db_manager();
        }
        "disconnect" => {
            ui.invoke_disconnect(ui.get_active_connection_id());
        }
        "toggle-theme" => {
            ui.invoke_toggle_theme();
        }
        "toggle-reduce-motion" => {
            ui.invoke_toggle_reduce_motion();
        }
        _ => {}
    }
}

pub(super) fn register_command_palette_callbacks(
    window: &crate::AppWindow,
    sidebar_state: Arc<Mutex<SidebarUiState>>,
    tx_cmd: mpsc::Sender<Command>,
    enc_key: [u8; 32],
) {
    let ui = window.global::<crate::UiState>();

    // command-palette-open: rebuild the full action list (including current
    // connections) then show the palette.
    {
        let window_weak = window.as_weak(); // clone required: on_command_palette_open closure
        let sidebar_state = Arc::clone(&sidebar_state); // clone required: capture for open callback
        ui.on_command_palette_open(move || {
            let Some(w) = window_weak.upgrade() else {
                return;
            };
            let ui = w.global::<crate::UiState>();
            let conns = current_connections(&sidebar_state);
            let actions = command_registry::build_actions(&conns);
            let items = palette_items_to_slint(&actions);
            ui.set_command_palette_query("".into());
            ui.set_command_palette_selected(0);
            ui.set_command_palette_items(Rc::new(slint::VecModel::from(items)).into());
            ui.set_show_command_palette(true);
        });
    }

    // command-palette-search: rebuild with current connections then fuzzy-filter.
    {
        let window_weak = window.as_weak(); // clone required: on_command_palette_search closure
        let sidebar_state = Arc::clone(&sidebar_state); // clone required: capture for search callback
        ui.on_command_palette_search(move |query| {
            let Some(w) = window_weak.upgrade() else {
                return;
            };
            let ui = w.global::<crate::UiState>();
            let conns = current_connections(&sidebar_state);
            let all = command_registry::build_actions(&conns);
            let matched = command_registry::filter_actions(&all, query.as_str());
            let items = palette_items_to_slint(&matched);
            ui.set_command_palette_items(Rc::new(slint::VecModel::from(items)).into());
            ui.set_command_palette_selected(0);
        });
    }

    // command-palette-execute: close palette then dispatch the chosen action.
    // "connect:<id>" entries send Command::Connect through tx_cmd; all other
    // ids are handled by dispatch_palette_command via Slint callbacks.
    {
        let window_weak = window.as_weak(); // clone required: on_command_palette_execute closure
        let sidebar_state = Arc::clone(&sidebar_state); // clone required: capture for execute callback
        let tx_cmd = tx_cmd.clone(); // clone required: capture for execute callback
        ui.on_command_palette_execute(move |id| {
            let Some(w) = window_weak.upgrade() else {
                return;
            };
            let ui = w.global::<crate::UiState>();
            ui.set_show_command_palette(false);
            ui.set_command_palette_query("".into());
            let id_str = id.as_str();
            if let Some(conn_id) = id_str.strip_prefix("connect:") {
                if ui.get_active_connection_id().as_str() == conn_id {
                    return;
                }
                let conn_cfg = with_sidebar(&sidebar_state, |sb| {
                    sb.config_connections
                        .iter()
                        .find(|c| c.id == conn_id)
                        .cloned()
                });
                if let Some(cc) = conn_cfg {
                    let conn = config_to_db_conn(&cc);
                    let password = conn
                        .password_encrypted
                        .as_ref()
                        .and_then(|enc| wf_config::crypto::decrypt(enc, &enc_key).ok());
                    send_cmd(&tx_cmd, Command::Connect(conn, password));
                }
            } else {
                dispatch_palette_command(&ui, id_str);
            }
        });
    }
}

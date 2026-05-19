use std::cell::RefCell;
use std::rc::Rc;

use slint::ComponentHandle;
use tokio::sync::mpsc;

use crate::app::command::Command;

use super::tabs_state::TabsState;
use super::undo::TextUndoState;
use super::{send_cmd, with_ui};

pub(super) fn register_history_callbacks(
    window: &crate::AppWindow,
    tx_cmd: mpsc::Sender<Command>,
    tabs_state: Rc<RefCell<TabsState>>,
    undo_state: Rc<TextUndoState>,
) {
    let ui = window.global::<crate::UiState>();

    // history-open: send SearchHistory with empty keyword to load recent 100 rows.
    {
        let tx = tx_cmd.clone(); // clone required: on_history_open closure
        ui.on_history_open(move || {
            send_cmd(
                &tx,
                Command::SearchHistory {
                    keyword: String::new(),
                    conn_id: None,
                },
            );
        });
    }

    // history-search: re-query on keyword change, debounced 150ms.
    {
        let tx = tx_cmd.clone(); // clone required: on_history_search closure
        let debounce: Rc<RefCell<Option<slint::Timer>>> = Rc::new(RefCell::new(None));
        let ww = window.as_weak();
        ui.on_history_search(move || {
            let keyword = ww
                .upgrade()
                .map(|w| w.global::<crate::UiState>().get_history_query().to_string())
                .unwrap_or_default();
            *debounce.borrow_mut() = None; // drop previous timer (cancels it)
            let tx = tx.clone(); // clone required: SingleShot timer closure
            let timer = slint::Timer::default();
            timer.start(
                slint::TimerMode::SingleShot,
                std::time::Duration::from_millis(150),
                move || {
                    send_cmd(
                        &tx,
                        Command::SearchHistory {
                            keyword: keyword.clone(),
                            conn_id: None,
                        },
                    );
                },
            );
            *debounce.borrow_mut() = Some(timer);
        });
    }

    // history-insert-sql: insert full SQL at the editor cursor position.
    {
        let ww = window.as_weak();
        let tabs_state = Rc::clone(&tabs_state); // clone required: on_history_insert_sql closure
        let undo_state = Rc::clone(&undo_state); // clone required: on_history_insert_sql closure
        ui.on_history_insert_sql(move |sql, cursor_pos| {
            with_ui(&ww, |ui| {
                let current = ui.get_editor_text().to_string();
                let tab_id = ui.get_editor_active_tab_id().to_string();
                undo_state.flush_before_programmatic_change(&mut tabs_state.borrow_mut(), &tab_id);
                tabs_state
                    .borrow_mut()
                    .push_undo_snapshot(&tab_id, current.clone());
                let pos = (cursor_pos as usize).min(current.len());
                let new_text = format!("{}{}{}", &current[..pos], sql, &current[pos..]);
                let new_cursor = (pos + sql.len()) as i32;
                *undo_state.last_known.borrow_mut() = new_text.clone();
                ui.set_editor_text(new_text.clone().into());
                ui.invoke_update_highlight(new_text.into());
                ui.set_editor_cursor_target(new_cursor);
                ui.set_show_history(false);
            });
        });
    }

    // history-execute-sql: replace editor text and execute immediately.
    {
        let ww = window.as_weak();
        let tabs_state = Rc::clone(&tabs_state); // clone required: on_history_execute_sql closure
        let undo_state = Rc::clone(&undo_state); // clone required: on_history_execute_sql closure
        ui.on_history_execute_sql(move |sql| {
            let Some(w) = ww.upgrade() else { return };
            let ui = w.global::<crate::UiState>();
            let current = ui.get_editor_text().to_string();
            let tab_id = ui.get_editor_active_tab_id().to_string();
            undo_state.flush_before_programmatic_change(&mut tabs_state.borrow_mut(), &tab_id);
            tabs_state.borrow_mut().push_undo_snapshot(&tab_id, current);
            *undo_state.last_known.borrow_mut() = sql.to_string();
            ui.set_editor_text(sql.clone());
            ui.invoke_update_highlight(sql.clone());
            ui.invoke_run_query(sql);
        });
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use wf_db::models::QueryExecution;

    use super::super::event::history_rows_to_slint;

    fn make_exec(sql: &str, duration_ms: u128, success: bool, ts: i64) -> QueryExecution {
        QueryExecution {
            id: 1,
            sql: sql.to_string(),
            duration_ms,
            success,
            error_message: None,
            timestamp: ts,
            connection_id: "c1".to_string(),
        }
    }

    #[test]
    fn history_rows_to_slint_should_format_timestamp_and_duration() {
        let rows = vec![
            make_exec("SELECT 1", 42, true, 1_700_000_000),
            make_exec("SELECT 2", 1500, false, 1_700_000_060),
        ];
        let entries = history_rows_to_slint(&rows);

        assert_eq!(entries[0].duration_text.as_str(), "42ms");
        assert_eq!(entries[1].duration_text.as_str(), "1.5s");
        assert!(entries[0].success);
        assert!(!entries[1].success);
        // timestamp should be non-empty (value depends on timezone, just check format)
        assert!(entries[0].timestamp_text.len() >= 16);
    }

    #[test]
    fn history_rows_to_slint_should_truncate_sql_preview_at_80_chars() {
        let long_sql = "SELECT ".to_string() + &"x".repeat(100);
        let rows = vec![make_exec(&long_sql, 10, true, 0)];
        let entries = history_rows_to_slint(&rows);
        assert!(entries[0].sql_preview.len() <= 80);
    }
}

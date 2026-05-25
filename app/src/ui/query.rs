use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use slint::{ComponentHandle, Model as _};
use tokio::sync::mpsc;

use crate::app::command::{Command, ConfigUpdate};
use crate::state::SharedState;

use super::appearance::{apply_highlight_spans, compute_highlight_spans};
use super::tabs_state::TabsState;
use super::undo::TextUndoState;
use super::{
    DEFAULT_COLUMN_WIDTH, OriginalQueryData, SharedOriginalData, check_safe_dml, send_cmd,
    set_status, with_ui,
};

/// Convert one raw result row (`Option<String>` cells) into a Slint `RowData`.
/// `None` → `RowCellData { value: "", is_null: true }`
/// `Some(s)` → `RowCellData { value: s, is_null: false }`
fn rows_to_ui(cells: &[Option<String>]) -> crate::RowData {
    let cell_data: Vec<crate::RowCellData> = cells
        .iter()
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
pub(crate) fn cells_to_tsv(cells: &[Option<String>]) -> String {
    cells
        .iter()
        .map(|c| c.as_deref().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\t")
}

/// Format `columns` + `rows` as a TSV string with a header line.
pub(crate) fn result_to_tsv(columns: &[&str], rows: &[Vec<Option<String>>]) -> String {
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
pub(crate) fn sort_rows(rows: &mut [&Vec<Option<String>>], col: usize, ascending: bool) {
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

/// Case-insensitive ASCII substring search with zero heap allocation.
///
/// `needle` must already be lowercased (via `str::to_lowercase`). Non-ASCII bytes
/// in `haystack` pass through `to_ascii_lowercase()` unchanged, so non-ASCII
/// uppercase letters (e.g. "É") will not match their lowercase equivalents ("é").
/// This is an acceptable trade-off for SQL data, which is almost entirely ASCII.
fn contains_case_insensitive_ascii(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let h = haystack.as_bytes();
    let n = needle.as_bytes();
    if n.len() > h.len() {
        return false;
    }
    h.windows(n.len())
        .any(|w| w.iter().zip(n).all(|(a, b)| a.to_ascii_lowercase() == *b))
}

/// Filter `rows` according to `query`:
///
/// * Empty query → return all rows.
/// * `col_name = 'value'` → exact match on the named column (case-insensitive column name).
///   NULL cells never match an `= 'value'` predicate.
/// * Anything else → case-insensitive substring match across all columns
///   (NULL cells are treated as empty string for substring matching).
///
/// Returns references into the original slice — no String cloning per cell.
pub(crate) fn filter_rows<'a>(
    columns: &[slint::SharedString],
    rows: &'a [Vec<Option<String>>],
    query: &str,
) -> Vec<&'a Vec<Option<String>>> {
    let query = query.trim();
    if query.is_empty() {
        return rows.iter().collect();
    }
    if let Some((col_name, value)) = parse_col_eq(query) {
        let col_idx = columns
            .iter()
            .position(|c| c.as_str().eq_ignore_ascii_case(&col_name));
        match col_idx {
            Some(idx) => rows
                .iter()
                .filter(|row| row.get(idx).is_some_and(|v| v.as_deref() == Some(value)))
                .collect(),
            None => vec![],
        }
    } else {
        let query_lower = query.to_lowercase();
        rows.iter()
            .filter(|row| {
                row.iter().any(|cell| {
                    contains_case_insensitive_ascii(cell.as_deref().unwrap_or(""), &query_lower)
                })
            })
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

pub(super) fn register_editor_callbacks(window: &crate::AppWindow, tx_cmd: mpsc::Sender<Command>) {
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
            tracing::debug!(sql = %sql, "on_run_query called");
            if check_safe_dml(&window_weak, &sql, "query") {
                tracing::debug!("on_run_query: blocked by safe_dml");
                return;
            }
            send_cmd(&tx_cmd, Command::RunQuery(sql.to_string()));
        });
    }
    {
        let tx_cmd = tx_cmd.clone(); // clone required: callback closure needs owned tx_cmd
        let window_weak = window.as_weak(); // clone required: check_safe_dml needs window ref
        ui.on_run_query_at_cursor(move |sql, cursor| {
            let stmt = wf_query::analyzer::extract_statement_at(sql.as_str(), cursor as usize);
            tracing::debug!(
                sql_len = sql.len(),
                cursor = cursor,
                stmt = %stmt,
                "on_run_query_at_cursor called"
            );
            if stmt.is_empty() {
                tracing::warn!("on_run_query_at_cursor: stmt is empty, no command sent");
                return;
            }
            if check_safe_dml(&window_weak, stmt, "cursor") {
                tracing::debug!("on_run_query_at_cursor: blocked by safe_dml");
                return;
            }
            send_cmd(&tx_cmd, Command::RunQuery(stmt.to_owned()));
        });
    }
    {
        let tx_cmd = tx_cmd.clone(); // clone required: callback closure needs owned tx_cmd
        ui.on_cancel_query(move || {
            send_cmd(&tx_cmd, Command::CancelQuery);
        });
    }
}

pub(super) fn register_formatter_callback(
    window: &crate::AppWindow,
    hl_model: Rc<slint::VecModel<crate::HighlightSpan>>,
    tabs_state: Rc<RefCell<TabsState>>,
    undo_state: Rc<TextUndoState>,
) {
    let ui = window.global::<crate::UiState>();

    // Registered first so the handler is ready before on_format_sql can fire.
    {
        let undo_state = Rc::clone(&undo_state); // clone required: on_format_sql_complete closure
        let hl_model = Rc::clone(&hl_model); // clone required: on_format_sql_complete closure
        let window_weak = window.as_weak();
        ui.on_format_sql_complete(move |tab_id, formatted| {
            with_ui(&window_weak, |ui| {
                ui.set_is_formatting(false);
                ui.set_status_message("".into());
                // Discard if user switched tabs while formatting was running.
                if ui.get_editor_active_tab_id() != tab_id {
                    return;
                }
                let formatted_str = formatted.to_string();
                let spans = compute_highlight_spans(&formatted_str);
                *undo_state.last_known.borrow_mut() = formatted_str.clone();
                ui.set_editor_text(formatted_str.into());
                apply_highlight_spans(&hl_model, spans);
            });
        });
    }

    // ── on_format_sql: fire-and-forget; only Send types cross the thread ──────
    {
        let undo_state = Rc::clone(&undo_state); // clone required: on_format_sql closure
        let tabs_state = Rc::clone(&tabs_state); // clone required: on_format_sql closure
        let window_weak = window.as_weak();
        ui.on_format_sql(move || {
            with_ui(&window_weak, |ui| {
                // Guard: ignore if a format is already in flight.
                if ui.get_is_formatting() {
                    return;
                }
                let text = ui.get_editor_text().to_string();
                let tab_id = ui.get_editor_active_tab_id().to_string();
                undo_state.flush_before_programmatic_change(&mut tabs_state.borrow_mut(), &tab_id);
                tabs_state
                    .borrow_mut()
                    .push_undo_snapshot(&tab_id, text.clone());
                ui.set_is_formatting(true);
                ui.set_status_message("Formatting\u{2026}".into());
                // Preserve original text in case spawn_blocking panics.
                let text_clone = text.clone(); // clone required: JoinError fallback
                let ww = window_weak.clone(); // clone required: tokio::spawn 'static
                tokio::spawn(async move {
                    let formatted =
                        tokio::task::spawn_blocking(move || wf_query::formatter::format_sql(&text))
                            .await
                            .unwrap_or(text_clone);
                    tracing::debug!(chars = formatted.len(), "format_sql complete");
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(w) = ww.upgrade() {
                            w.global::<crate::UiState>()
                                .invoke_format_sql_complete(tab_id.into(), formatted.into());
                        }
                    });
                });
            });
        });
    }
}

const CSV_DEFAULT_FILENAME: &str = "query_result.csv";
const JSON_DEFAULT_FILENAME: &str = "query_result.json";
const INSERT_SQL_DEFAULT_FILENAME: &str = "query_result.sql";

pub(super) fn register_export_callbacks(
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
                    (cols, Arc::clone(&d.rows)) // clone required: tokio::spawn needs 'static
                })
            };
            let Some((columns, rows)) = snapshot else {
                return;
            };
            let window_weak = window_weak.clone(); // clone required: tokio::spawn needs 'static
            tokio::spawn(async move {
                let Some(handle) = rfd::AsyncFileDialog::new()
                    .set_title("Save CSV")
                    .set_file_name(CSV_DEFAULT_FILENAME)
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
                    (cols, Arc::clone(&d.rows)) // clone required: tokio::spawn needs 'static
                })
            };
            let Some((columns, rows)) = snapshot else {
                return;
            };
            let window_weak = window_weak.clone(); // clone required: tokio::spawn needs 'static
            tokio::spawn(async move {
                let Some(handle) = rfd::AsyncFileDialog::new()
                    .set_title("Save JSON")
                    .set_file_name(JSON_DEFAULT_FILENAME)
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
                    (cols, Arc::clone(&d.rows)) // clone required: tokio::spawn needs 'static
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
                    .set_file_name(INSERT_SQL_DEFAULT_FILENAME)
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

// ── Result callbacks ──────────────────────────────────────────────────────────

pub(super) fn register_result_callbacks(
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
                let rows: Vec<crate::RowData> =
                    filtered.into_iter().map(|r| rows_to_ui(r)).collect();
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
                let mut rows: Vec<&Vec<Option<String>>> = data.rows.iter().collect();
                if let Some(col) = data.sort_col {
                    sort_rows(&mut rows, col, data.sort_asc);
                }
                let row_count = rows.len() as i32;
                let ui_rows: Vec<crate::RowData> =
                    rows.into_iter().map(|r| rows_to_ui(r)).collect();
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
                // filter_rows returns references into orig's data, so sort and conversion
                // must happen inside the lock block before the MutexGuard drops.
                let (new_col, new_asc, row_count, ui_rows) = {
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
                    let mut filtered = filter_rows(&data.columns, &data.rows, &filter_q);
                    if let Some(c) = new_col {
                        sort_rows(&mut filtered, c, new_asc);
                    }
                    let row_count = filtered.len() as i32;
                    let ui_rows: Vec<crate::RowData> =
                        filtered.into_iter().map(|r| rows_to_ui(r)).collect();
                    (new_col, new_asc, row_count, ui_rows)
                };
                ui.set_result_rows(Rc::new(slint::VecModel::from(ui_rows)).into());
                ui.set_result_row_count(row_count);
                ui.set_result_sort_col(new_col.map(|c| c as i32).unwrap_or(-1));
                ui.set_result_sort_asc(new_asc);
            });
        });
    }
}

pub(super) fn handle_query_finished(
    result: wf_db::models::QueryResult,
    ww: slint::Weak<crate::AppWindow>,
    original_data: SharedOriginalData,
) {
    // Build Send data outside invoke_from_event_loop (Rc<VecModel> is not Send).
    let col_count = result.columns.len();
    let columns: Vec<slint::SharedString> =
        result.columns.iter().map(|c| c.clone().into()).collect();
    let raw_rows = Arc::new(result.rows);
    let row_count = result.row_count as i32;
    let exec_ms = result.execution_time_ms;
    {
        let mut orig = original_data.lock().unwrap_or_else(|p| p.into_inner());
        *orig = Some(OriginalQueryData {
            columns: columns.clone(),
            rows: Arc::clone(&raw_rows),
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
            let rows: Vec<crate::RowData> = raw_rows.iter().map(|r| rows_to_ui(r)).collect();
            ui.set_result_rows(Rc::new(slint::VecModel::from(rows)).into());
            ui.set_result_row_count(row_count);
            ui.set_result_total_rows(row_count);
            let widths: Vec<f32> = vec![DEFAULT_COLUMN_WIDTH; col_count];
            let total_w = col_count as f32 * DEFAULT_COLUMN_WIDTH;
            ui.set_result_col_widths(Rc::new(slint::VecModel::from(widths)).into());
            ui.set_result_total_col_width(total_w);
            ui.set_status_message(
                rust_i18n::t!("status.query_finished", ms = exec_ms, rows = row_count)
                    .to_string()
                    .into(),
            );
            let threshold = ui.get_slow_query_threshold_ms();
            if threshold > 0 && exec_ms >= threshold as u128 {
                ui.set_slow_query_warning(
                    rust_i18n::t!("status.slow_query", ms = exec_ms)
                        .to_string()
                        .into(),
                );
            } else {
                ui.set_slow_query_warning("".into());
            }
            ui.set_result_panel_open(true);
        });
    });
}

pub(super) fn handle_table_data_loaded(
    _tab_id: String,
    result: wf_db::models::QueryResult,
    ww: slint::Weak<crate::AppWindow>,
) {
    let col_count = result.columns.len();
    let columns: Vec<slint::SharedString> =
        result.columns.iter().map(|c| c.clone().into()).collect();
    let raw_rows = result.rows;
    let row_count = result.row_count as i32;
    // clone required: invoke_from_event_loop closure must be 'static
    let _ = slint::invoke_from_event_loop(move || {
        with_ui(&ww, move |ui| {
            ui.set_tv_data_loading(false);
            ui.set_tv_data_error("".into());
            let col_model = Rc::new(slint::VecModel::from(columns));
            ui.set_result_columns(col_model.into());
            let rows: Vec<crate::RowData> = raw_rows.iter().map(|r| rows_to_ui(r)).collect();
            ui.set_result_rows(Rc::new(slint::VecModel::from(rows)).into());
            ui.set_result_row_count(row_count);
            let widths: Vec<f32> = vec![DEFAULT_COLUMN_WIDTH; col_count];
            let total_w = col_count as f32 * DEFAULT_COLUMN_WIDTH;
            ui.set_result_col_widths(Rc::new(slint::VecModel::from(widths)).into());
            ui.set_result_total_col_width(total_w);
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ss(s: &str) -> slint::SharedString {
        s.into()
    }

    fn sv(s: &str) -> Option<String> {
        Some(s.to_string())
    }

    // ── filter_rows ───────────────────────────────────────────────────────────

    #[test]
    fn filter_rows_should_return_all_when_query_is_empty_string() {
        let cols = vec![ss("id"), ss("name")];
        let rows = vec![vec![sv("1"), sv("Alice")], vec![sv("2"), sv("Bob")]];
        assert_eq!(filter_rows(&cols, &rows, "").len(), 2);
    }

    #[test]
    fn filter_rows_should_return_all_when_query_is_whitespace() {
        let cols = vec![ss("id"), ss("name")];
        let rows = vec![vec![sv("1"), sv("Alice")], vec![sv("2"), sv("Bob")]];
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
        let rows = vec![vec![None], vec![sv("Alice")]];
        let result = filter_rows(&cols, &rows, "Alice");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0][0].as_deref(), Some("Alice"));
    }

    #[test]
    fn filter_rows_should_return_all_on_empty_query() {
        let cols = vec![ss("id"), ss("name")];
        let rows = vec![vec![sv("1"), sv("Alice")], vec![sv("2"), sv("Bob")]];
        assert_eq!(filter_rows(&cols, &rows, "").len(), 2);
    }

    #[test]
    fn filter_rows_should_match_case_insensitively() {
        let cols = vec![ss("name")];
        let rows = vec![vec![sv("Alice")], vec![sv("BOB")], vec![sv("charlie")]];
        let r = filter_rows(&cols, &rows, "ALICE");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0][0].as_deref(), Some("Alice"));
        let r = filter_rows(&cols, &rows, "bob");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0][0].as_deref(), Some("BOB"));
    }

    // ── sort_rows ─────────────────────────────────────────────────────────────

    #[test]
    fn sort_rows_should_sort_strings_ascending() {
        let r0 = vec![sv("banana")];
        let r1 = vec![sv("apple")];
        let r2 = vec![sv("cherry")];
        let mut rows = vec![&r0, &r1, &r2];
        sort_rows(&mut rows, 0, true);
        assert_eq!(rows[0][0].as_deref(), Some("apple"));
        assert_eq!(rows[1][0].as_deref(), Some("banana"));
        assert_eq!(rows[2][0].as_deref(), Some("cherry"));
    }

    #[test]
    fn sort_rows_should_sort_strings_descending() {
        let r0 = vec![sv("banana")];
        let r1 = vec![sv("apple")];
        let r2 = vec![sv("cherry")];
        let mut rows = vec![&r0, &r1, &r2];
        sort_rows(&mut rows, 0, false);
        assert_eq!(rows[0][0].as_deref(), Some("cherry"));
        assert_eq!(rows[1][0].as_deref(), Some("banana"));
        assert_eq!(rows[2][0].as_deref(), Some("apple"));
    }

    #[test]
    fn sort_rows_should_sort_numerically_when_values_are_numbers() {
        let r0 = vec![sv("10")];
        let r1 = vec![sv("2")];
        let r2 = vec![sv("20")];
        let mut rows = vec![&r0, &r1, &r2];
        sort_rows(&mut rows, 0, true);
        assert_eq!(rows[0][0].as_deref(), Some("2"));
        assert_eq!(rows[1][0].as_deref(), Some("10"));
        assert_eq!(rows[2][0].as_deref(), Some("20"));
    }

    #[test]
    fn sort_rows_should_put_nulls_last_ascending() {
        let r0: Vec<Option<String>> = vec![None];
        let r1 = vec![sv("b")];
        let r2 = vec![sv("a")];
        let mut rows = vec![&r0, &r1, &r2];
        sort_rows(&mut rows, 0, true);
        assert_eq!(rows[0][0].as_deref(), Some("a"));
        assert_eq!(rows[1][0].as_deref(), Some("b"));
        assert!(rows[2][0].is_none());
    }

    #[test]
    fn sort_rows_should_put_nulls_last_descending() {
        let r0: Vec<Option<String>> = vec![None];
        let r1 = vec![sv("b")];
        let r2 = vec![sv("a")];
        let mut rows = vec![&r0, &r1, &r2];
        sort_rows(&mut rows, 0, false);
        assert_eq!(rows[0][0].as_deref(), Some("b"));
        assert_eq!(rows[1][0].as_deref(), Some("a"));
        assert!(rows[2][0].is_none());
    }

    // ── cells_to_tsv / result_to_tsv ──────────────────────────────────────────

    #[test]
    fn cells_to_tsv_should_join_values_with_tabs() {
        let cells = vec![sv("a"), sv("b"), sv("c")];
        assert_eq!(cells_to_tsv(&cells), "a\tb\tc");
    }

    #[test]
    fn cells_to_tsv_should_render_null_as_empty_string() {
        let cells = vec![sv("a"), None, sv("c")];
        assert_eq!(cells_to_tsv(&cells), "a\t\tc");
    }

    #[test]
    fn cells_to_tsv_should_handle_empty_row() {
        let cells: Vec<Option<String>> = vec![];
        assert_eq!(cells_to_tsv(&cells), "");
    }

    #[test]
    fn result_to_tsv_should_include_header_and_rows() {
        let cols = vec!["id", "name"];
        let rows = vec![vec![sv("1"), sv("Alice")], vec![sv("2"), sv("Bob")]];
        let tsv = result_to_tsv(&cols, &rows);
        assert_eq!(tsv, "id\tname\n1\tAlice\n2\tBob");
    }

    #[test]
    fn result_to_tsv_should_render_null_cells_as_empty_string() {
        let cols = vec!["id", "name"];
        let rows = vec![vec![sv("1"), None]];
        let tsv = result_to_tsv(&cols, &rows);
        assert_eq!(tsv, "id\tname\n1\t");
    }

    #[test]
    fn result_to_tsv_should_produce_header_only_when_no_rows() {
        let cols = vec!["id", "name"];
        let rows: Vec<Vec<Option<String>>> = vec![];
        let tsv = result_to_tsv(&cols, &rows);
        assert_eq!(tsv, "id\tname");
    }
}

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, Model as _};
use tokio::sync::mpsc;

use wf_config::models::Theme;
use wf_history::session::SessionService;

use crate::app::command::{Command, ConfigUpdate};

use super::tabs_state::TabsState;
use super::undo::TextUndoState;
use super::{send_cmd, tabs_state, with_ui};

/// Tokenize `sql` and return Slint-typed `HighlightSpan` values for rendering.
pub(super) fn compute_highlight_spans(sql: &str) -> Vec<crate::HighlightSpan> {
    wf_query::highlight::highlight(sql)
        .into_iter()
        .map(|s| crate::HighlightSpan {
            line: s.line,
            col: s.col,
            text: s.text.into(),
            kind: s.kind,
        })
        .collect()
}

/// Update a persistent highlight VecModel in-place to avoid destroying and
/// recreating all overlay Text elements on every keystroke.
///
/// The model grows monotonically — rows are never removed. Excess rows (when
/// span count decreases) are overwritten with a sentinel span whose `text` is
/// empty and therefore renders nothing. This replaces `model.remove()` calls,
/// which send `row_removed` notifications that can cause Text elements to be
/// destroyed and recreated in a separate compositing step from the TextInput
/// cursor-position update, producing a one-frame visual misalignment.
pub(super) fn apply_highlight_spans(
    model: &Rc<slint::VecModel<crate::HighlightSpan>>,
    spans: Vec<crate::HighlightSpan>,
) {
    let n = model.row_count();
    let m = spans.len();
    for (i, span) in spans.iter().enumerate().take(n.min(m)) {
        model.set_row_data(i, span.clone());
    }
    for span in spans.into_iter().skip(n) {
        model.push(span);
    }
    // Clear excess rows with a sentinel rather than removing them.
    // kind=4 is the gap-fill (identifier) value; empty text renders nothing.
    let sentinel = crate::HighlightSpan {
        line: 0,
        col: 0,
        text: "".into(),
        kind: 4,
    };
    for i in m..n {
        model.set_row_data(i, sentinel.clone());
    }
}

pub(super) fn register_theme_callback(
    window: &crate::AppWindow,
    state: crate::state::SharedState,
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

pub(super) fn register_reduce_motion_callback(
    window: &crate::AppWindow,
    tx_cmd: mpsc::Sender<Command>,
) {
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

// ── Window lifecycle ──────────────────────────────────────────────────────────

pub(super) fn register_close_handler(
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

// ── Menu bar callbacks ────────────────────────────────────────────────────────

pub(super) fn register_menu_callbacks(window: &crate::AppWindow, tx_cmd: mpsc::Sender<Command>) {
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
            if super::check_safe_dml(&window_weak, &sql, "all") {
                return;
            }
            send_cmd(&tx_cmd, Command::RunAll(sql.to_string()));
        });
    }
}

// ── Language callback ─────────────────────────────────────────────────────────

pub(super) fn register_language_callback(window: &crate::AppWindow, tx_cmd: mpsc::Sender<Command>) {
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

// ── Editor preferences callbacks ──────────────────────────────────────────────

pub(super) fn register_editor_prefs_callbacks(
    window: &crate::AppWindow,
    tx_cmd: mpsc::Sender<Command>,
    tabs_state: Rc<RefCell<TabsState>>,
    undo_state: Rc<TextUndoState>,
) {
    let ui = window.global::<crate::UiState>();

    // Tab key: insert spaces at cursor, then update editor text + cursor target.
    // Runs on the UI thread (same as format_sql); text is committed before the
    // cursor target change so set-selection-offsets validates against the new text.
    let window_weak = window.as_weak(); // clone required: on_tab_key_pressed closure
    ui.on_tab_key_pressed(move |cursor| {
        let Some(window) = window_weak.upgrade() else {
            return;
        };
        let ui = window.global::<crate::UiState>();
        let text = ui.get_editor_text().to_string();
        let tab_id = ui.get_editor_active_tab_id().to_string();
        let tab_width = (ui.get_tab_width() as usize).max(1);
        let cursor = cursor as usize;

        if cursor > text.len() || !text.is_char_boundary(cursor) {
            return;
        }

        undo_state.flush_before_programmatic_change(&mut tabs_state.borrow_mut(), &tab_id);
        tabs_state
            .borrow_mut()
            .push_undo_snapshot(&tab_id, text.clone());

        let spaces = " ".repeat(tab_width);
        let mut new_text = text;
        new_text.insert_str(cursor, &spaces);
        let new_cursor = (cursor + tab_width) as i32;

        *undo_state.last_known.borrow_mut() = new_text.clone();
        let shared: slint::SharedString = new_text.into();
        ui.set_editor_text(shared.clone());
        ui.set_editor_cursor_target(new_cursor);
        ui.invoke_update_highlight(shared);
    });

    {
        let tx_cmd = tx_cmd.clone(); // clone required: on_set_tab_width closure
        ui.on_set_tab_width(move |width| {
            send_cmd(
                &tx_cmd,
                Command::UpdateConfig(ConfigUpdate::TabWidth(width as u32)),
            );
        });
    }

    {
        let tx_cmd = tx_cmd.clone(); // clone required: on_set_query_timeout_secs closure
        ui.on_set_query_timeout_secs(move |secs| {
            send_cmd(
                &tx_cmd,
                Command::UpdateConfig(ConfigUpdate::QueryTimeout(secs.max(0) as u64)),
            );
        });
    }

    {
        let tx_cmd = tx_cmd.clone(); // clone required: on_set_slow_query_threshold_ms closure
        ui.on_set_slow_query_threshold_ms(move |ms| {
            send_cmd(
                &tx_cmd,
                Command::UpdateConfig(ConfigUpdate::SlowQueryThreshold(ms.max(0) as u64)),
            );
        });
    }

    {
        let tx_cmd = tx_cmd.clone(); // clone required: on_set_font_family closure
        ui.on_set_font_family(move |family| {
            send_cmd(
                &tx_cmd,
                Command::UpdateConfig(ConfigUpdate::FontFamily(family.to_string())),
            );
        });
    }

    ui.on_set_font_size(move |size| {
        send_cmd(
            &tx_cmd,
            Command::UpdateConfig(ConfigUpdate::FontSize(size.max(1) as u32)),
        );
    });
}

// ── Syntax highlight callback ─────────────────────────────────────────────────

pub(super) fn register_highlight_callbacks(
    window: &crate::AppWindow,
    hl_model: Rc<slint::VecModel<crate::HighlightSpan>>,
) {
    let ui = window.global::<crate::UiState>();
    ui.on_update_highlight(move |sql| {
        let spans = compute_highlight_spans(&sql);
        apply_highlight_spans(&hl_model, spans);
    });
}

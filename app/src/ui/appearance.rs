use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, Model as _};
use tokio::sync::mpsc;

use wf_config::models::Theme;
use wf_history::session::SessionService;

use crate::app::command::{Command, ConfigUpdate};

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
    while model.row_count() > m {
        model.remove(model.row_count() - 1);
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

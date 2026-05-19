use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use slint::ComponentHandle;

use crate::ui::tabs_state::TabsState;

use super::with_ui;

/// Shared state for the text undo/redo debounce logic.
/// Lives on the UI thread; always accessed behind `Rc<RefCell<>>`.
pub struct TextUndoState {
    /// Text as of the last observed state (after any keystroke or programmatic change).
    pub last_known: RefCell<String>,
    /// Text that existed just before the current typing burst began, plus the tab it belongs to.
    /// `None` when no burst is in progress.
    pub burst_start: RefCell<Option<(String, String)>>,
    /// The active debounce timer; `None` means no burst is pending.
    pub debounce: RefCell<Option<slint::Timer>>,
}

impl TextUndoState {
    pub fn new(initial_text: String) -> Rc<Self> {
        Rc::new(Self {
            last_known: RefCell::new(initial_text),
            burst_start: RefCell::new(None),
            debounce: RefCell::new(None),
        })
    }

    /// Cancel any pending debounce timer and flush its burst_start snapshot
    /// into the undo stack before a programmatic `set_editor_text` call.
    ///
    /// Call this BEFORE pushing `current_text` as a new snapshot and calling
    /// `set_editor_text`.  After the call, update `last_known` to the new text.
    pub fn flush_before_programmatic_change(
        &self,
        tabs_state: &mut TabsState,
        current_tab_id: &str,
    ) {
        // Cancel debounce (prevents stale burst firing after programmatic change).
        *self.debounce.borrow_mut() = None;
        // Flush any in-progress burst snapshot.
        if let Some((tab_id, before_text)) = self.burst_start.borrow_mut().take() {
            tabs_state.push_undo_snapshot(&tab_id, before_text);
        }
        let _ = current_tab_id; // used by callers to push the programmatic snapshot
    }
}

pub(super) fn register_undo_callbacks(
    window: &crate::AppWindow,
    tabs_state: Rc<RefCell<TabsState>>,
    undo_state: Rc<TextUndoState>,
) {
    let ui = window.global::<crate::UiState>();
    let ww = window.as_weak();

    // ── text-changed: debounce 500ms, snapshot pre-burst text ────────────────
    {
        let undo_state2 = Rc::clone(&undo_state); // clone required: on_text_changed closure
        let tabs_state2 = Rc::clone(&tabs_state); // clone required: on_text_changed closure
        let ww2 = ww.clone(); // clone required: on_text_changed closure
        ui.on_text_changed(move |new_text| {
            let new_text = new_text.to_string();
            let is_first_of_burst = undo_state2.debounce.borrow().is_none();
            if is_first_of_burst {
                // Capture the text before this burst starts.
                let before = undo_state2.last_known.borrow().clone();
                let tab_id = ww2
                    .upgrade()
                    .map(|w| {
                        w.global::<crate::UiState>()
                            .get_editor_active_tab_id()
                            .to_string()
                    })
                    .unwrap_or_default();
                *undo_state2.burst_start.borrow_mut() = Some((tab_id, before));
            }
            *undo_state2.last_known.borrow_mut() = new_text;

            // Reset debounce timer.
            *undo_state2.debounce.borrow_mut() = None; // cancel previous
            let undo_state3 = Rc::clone(&undo_state2); // clone required: SingleShot timer closure
            let tabs_state3 = Rc::clone(&tabs_state2); // clone required: SingleShot timer closure
            let timer = slint::Timer::default();
            timer.start(
                slint::TimerMode::SingleShot,
                Duration::from_millis(500),
                move || {
                    let Some((tab_id, before_text)) = undo_state3.burst_start.borrow_mut().take()
                    else {
                        return;
                    };
                    tabs_state3
                        .borrow_mut()
                        .push_undo_snapshot(&tab_id, before_text);
                    *undo_state3.debounce.borrow_mut() = None;
                },
            );
            *undo_state2.debounce.borrow_mut() = Some(timer);
        });
    }

    // ── editor-undo: restore previous text snapshot ───────────────────────────
    {
        let undo_state2 = Rc::clone(&undo_state); // clone required: on_editor_undo closure
        let tabs_state2 = Rc::clone(&tabs_state); // clone required: on_editor_undo closure
        let ww2 = ww.clone(); // clone required: on_editor_undo closure
        ui.on_editor_undo(move || {
            with_ui(&ww2, |ui| {
                let tab_id = ui.get_editor_active_tab_id().to_string();
                let current = ui.get_editor_text().to_string();

                // Flush any in-progress burst into the stack first so
                // Ctrl+Z after mid-burst can step back through it.
                {
                    let mut ts = tabs_state2.borrow_mut();
                    if let Some((burst_tab, before_text)) =
                        undo_state2.burst_start.borrow_mut().take()
                    {
                        ts.push_undo_snapshot(&burst_tab, before_text);
                    }
                }
                *undo_state2.debounce.borrow_mut() = None;

                let Some(prev) = tabs_state2.borrow_mut().text_undo(&tab_id, current) else {
                    return;
                };
                *undo_state2.last_known.borrow_mut() = prev.clone();
                ui.set_editor_text(prev.clone().into());
                ui.invoke_update_highlight(prev.into());
            });
        });
    }

    // ── editor-redo: restore next text snapshot ───────────────────────────────
    {
        let undo_state2 = Rc::clone(&undo_state); // clone required: on_editor_redo closure
        let tabs_state2 = Rc::clone(&tabs_state); // clone required: on_editor_redo closure
        let ww2 = ww.clone(); // clone required: on_editor_redo closure
        ui.on_editor_redo(move || {
            with_ui(&ww2, |ui| {
                let tab_id = ui.get_editor_active_tab_id().to_string();
                let current = ui.get_editor_text().to_string();

                *undo_state2.debounce.borrow_mut() = None;
                *undo_state2.burst_start.borrow_mut() = None;

                let Some(next) = tabs_state2.borrow_mut().text_redo(&tab_id, current) else {
                    return;
                };
                *undo_state2.last_known.borrow_mut() = next.clone();
                ui.set_editor_text(next.clone().into());
                ui.invoke_update_highlight(next.into());
            });
        });
    }
}

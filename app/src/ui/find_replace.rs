use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use slint::ComponentHandle;
use wf_history::find_history::FindHistoryService;

use super::{FindState, HistorySnapshot, SharedHistorySnapshot};

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

pub(super) fn register_find_replace_callbacks(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_matches_should_return_empty_for_empty_query() {
        assert!(compute_matches("hello world", "", false, false).is_empty());
    }

    #[test]
    fn compute_matches_should_find_all_occurrences() {
        let m = compute_matches("abcabc", "abc", true, false);
        assert_eq!(m, vec![(0, 3), (3, 6)]);
    }

    #[test]
    fn compute_matches_should_be_case_insensitive_by_default() {
        let m = compute_matches("Hello HELLO hello", "hello", false, false);
        assert_eq!(m.len(), 3);
    }

    #[test]
    fn compute_matches_should_respect_case_sensitive_flag() {
        let m = compute_matches("Hello HELLO hello", "hello", true, false);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0], (12, 17));
    }

    #[test]
    fn compute_matches_should_support_regex_pattern() {
        let m = compute_matches("foo123bar456", r"\d+", false, true);
        assert_eq!(m, vec![(3, 6), (9, 12)]);
    }

    #[test]
    fn compute_matches_should_return_empty_for_invalid_regex() {
        assert!(compute_matches("hello", "[invalid", false, true).is_empty());
    }

    #[test]
    fn compute_matches_should_return_empty_for_no_match() {
        assert!(compute_matches("hello", "xyz", false, false).is_empty());
    }
}

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use slint::ComponentHandle;
use tokio::sync::mpsc;

use crate::app::command::Command;

use super::{COMPLETION_DEBOUNCE_MS, send_cmd, with_ui};

fn completion_kind_label(kind: &wf_completion::CompletionKind) -> &'static str {
    use wf_completion::CompletionKind::*;
    match kind {
        Table => "Table",
        Column => "Column",
        Keyword => "Keyword",
        Operator => "Operator",
        Schema => "Schema",
        View => "View",
    }
}

/// Returns the byte offset of the first character of the word immediately
/// before `cursor_pos` in `text`.  Handles dot-qualified names (e.g. `u.em`)
/// by only considering the segment after the last `.`.
pub(crate) fn find_prefix_start(text: &str, cursor_pos: usize) -> usize {
    let pos = cursor_pos.min(text.len());
    let before = &text[..pos];
    let search_start = before.rfind('.').map(|i| i + 1).unwrap_or(0);
    let prefix_len = before[search_start..]
        .bytes()
        .rev()
        .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
        .count();
    pos - prefix_len
}

/// Returns `true` when `text` ends at a position where showing the next
/// completion popup would be misleading — specifically after terminal SQL
/// expressions: IS NULL, IS NOT NULL, TRUE, FALSE, ASC, DESC, string
/// literals (ending `'`), or numeric literals.
///
/// This suppresses the auto-retrigger after, e.g., `IS NOT NULL` so the
/// editor does not immediately show column candidates for the next `AND`
/// clause; the user can still invoke Ctrl+Space explicitly if needed.
pub(crate) fn is_terminal_expression(text: &str) -> bool {
    let t = text.trim_end();
    // Last whole-word token (alphanumeric + underscore run after final separator).
    let last_word_start = t
        .rfind(|c: char| !c.is_alphanumeric() && c != '_')
        .map(|i| i + 1)
        .unwrap_or(0);
    let last_word = t[last_word_start..].to_ascii_uppercase();
    if matches!(
        last_word.as_str(),
        "NULL" | "TRUE" | "FALSE" | "ASC" | "DESC"
    ) {
        return true;
    }
    // String literal or numeric literal.
    t.ends_with('\'') || t.ends_with('"') || t.chars().last().is_some_and(|c| c.is_ascii_digit())
}

/// Returns `true` if `sql` contains a FROM clause (case-insensitive).
/// Used to decide whether to auto-append `FROM <table>` after a column acceptance.
pub(crate) fn sql_has_from(sql: &str) -> bool {
    let upper = sql.to_ascii_uppercase();
    upper.contains(" FROM ")
        || upper.contains("\nFROM ")
        || upper.contains("\tFROM ")
        || upper.starts_with("FROM ")
}

pub(super) fn register_completion_callbacks(
    window: &crate::AppWindow,
    tx_cmd: mpsc::Sender<Command>,
) {
    let ui = window.global::<crate::UiState>();

    // Debounced path (text-change → 300 ms → FetchCompletion).
    // Dropping the previous timer stops it — each keystroke resets the window.
    let debounce: Rc<RefCell<Option<slint::Timer>>> = Rc::new(RefCell::new(None));
    {
        let debounce = debounce.clone(); // clone required: on_fetch_completion closure
        let tx_cmd = tx_cmd.clone(); // clone required: on_fetch_completion closure
        let window_weak = window.as_weak(); // clone required: prefix-check close path
        ui.on_fetch_completion(move |sql, cursor_pos| {
            *debounce.borrow_mut() = None; // drop previous timer → cancels it

            let s = sql.as_str();
            let pos = (cursor_pos as usize).min(s.len());

            // Close the popup immediately when there is no word prefix before
            // the cursor — covers Backspace-to-empty, space, semicolon, etc.
            // Without this, the debounce would fire and NextClause candidates
            // (WHERE / GROUP BY …) would reopen the popup 300 ms later.
            if find_prefix_start(s, pos) == pos {
                with_ui(&window_weak, |ui| ui.set_completion_visible(false));
                return;
            }

            let tx = tx_cmd.clone(); // clone required: Timer callback
            let sql = sql.to_string();
            let timer = slint::Timer::default();
            timer.start(
                slint::TimerMode::SingleShot,
                Duration::from_millis(COMPLETION_DEBOUNCE_MS),
                move || {
                    let sql = sql.clone();
                    send_cmd(&tx, Command::FetchCompletion(sql, cursor_pos as usize));
                },
            );
            *debounce.borrow_mut() = Some(timer);
        });
    }

    // Immediate path (Ctrl+Space → FetchCompletion without delay).
    {
        ui.on_trigger_completion(move |sql, cursor_pos| {
            send_cmd(
                &tx_cmd,
                Command::FetchCompletion(sql.to_string(), cursor_pos as usize),
            );
        });
    }
}

pub(super) fn register_completion_accept_callback(window: &crate::AppWindow) {
    let ui = window.global::<crate::UiState>();
    let window_weak = window.as_weak(); // clone required: on_accept_completion closure
    ui.on_accept_completion(
        move |insert_text, cursor_pos, cursor_offset_val, table_name| {
            with_ui(&window_weak, move |ui| {
                let current = ui.get_editor_text().to_string();
                let pos = (cursor_pos as usize).min(current.len());
                let mut prefix_start = find_prefix_start(&current, pos);
                // When accepting a disambiguated column candidate (table_name is set), the
                // user may have typed "colname tableprefix" with a space.  Extend the
                // replacement range backward to cover the entire "colname tableprefix" so the
                // insertion replaces both words, not just the current word after the space.
                let table_name_str = table_name.to_string();
                if !table_name_str.is_empty() {
                    let before_prefix = &current[..prefix_start];
                    let pattern = format!("{} ", insert_text.as_str());
                    if before_prefix.ends_with(&pattern) {
                        let extended = prefix_start - pattern.len();
                        let at_boundary = extended == 0
                            || matches!(
                                current.as_bytes().get(extended - 1),
                                Some(b' ') | Some(b'\t') | Some(b'\n')
                            );
                        if at_boundary {
                            prefix_start = extended;
                        }
                    }
                }

                // If the accepted text is a SQL keyword (FROM, WHERE, AND …), treat it as
                // a plain insertion even inside a SELECT list — no comma should be added.
                let is_keyword = wf_completion::parser::is_sql_keyword(insert_text.as_str());
                let in_select = !is_keyword && wf_completion::parser::in_select_list(&current, pos);

                let (new_text, new_cursor): (String, i32) = if in_select {
                    // In SELECT list: auto-insert ", " between columns.
                    let trimmed = current[..prefix_start].trim_end_matches([' ', '\t']);
                    let last_char = trimmed.chars().last();
                    let last_word_start = trimmed
                        .rfind(|c: char| !c.is_alphanumeric() && c != '_')
                        .map(|i| i + 1)
                        .unwrap_or(0);
                    let last_word = trimmed[last_word_start..].to_ascii_uppercase();
                    let needs_comma = !matches!(last_char, None | Some(',') | Some('('))
                        && !matches!(last_word.as_str(), "SELECT" | "DISTINCT");
                    if needs_comma {
                        let text = format!("{}, {}{}", trimmed, insert_text, &current[pos..]);
                        let cur = (trimmed.len() + 2 + insert_text.len()) as i32;
                        (text, cur)
                    } else {
                        let text = format!(
                            "{}{}{}",
                            &current[..prefix_start],
                            insert_text,
                            &current[pos..]
                        );
                        let cur = (prefix_start + insert_text.len()) as i32;
                        (text, cur)
                    }
                } else {
                    // Determine whether to replace the typed prefix or insert at cursor.
                    // When the accepted text is unrelated to the prefix (e.g. user finished
                    // typing "users" and now accepts a NextClause keyword like "WHERE"),
                    // insert at the cursor position with a leading space rather than
                    // overwriting the table/column name.
                    let prefix_word = &current[prefix_start..pos];
                    let (actual_start, add_leading_space) = if prefix_word.is_empty() {
                        // Cursor is at whitespace or string start — plain insert.
                        (pos, false)
                    } else if insert_text
                        .as_str()
                        .to_ascii_uppercase()
                        .starts_with(&prefix_word.to_ascii_uppercase())
                    {
                        // Prefix is a partial match of insert_text — replace it.
                        (prefix_start, false)
                    } else {
                        // Prefix is unrelated (e.g. "users" + "WHERE") — insert at cursor.
                        let needs_space =
                            !current[..pos].ends_with(|c: char| c.is_ascii_whitespace());
                        (pos, needs_space)
                    };
                    let leading = if add_leading_space { " " } else { "" };
                    let text = format!(
                        "{}{}{}{}",
                        &current[..actual_start],
                        leading,
                        insert_text,
                        &current[pos..]
                    );
                    let cur = if cursor_offset_val > 0 {
                        actual_start as i32 + add_leading_space as i32 + cursor_offset_val
                    } else {
                        (actual_start + leading.len() + insert_text.len()) as i32
                    };
                    (text, cur)
                };

                // Auto-append FROM <table> when a column with a known table was accepted
                // inside a SELECT list that has no FROM clause yet.
                let appended_from =
                    in_select && !table_name_str.is_empty() && !sql_has_from(&current);
                let (final_text, final_cursor) = if appended_from {
                    let appended = format!("{} FROM {}", new_text.trim_end(), table_name_str);
                    let cur = appended.len() as i32;
                    (appended, cur)
                } else {
                    (new_text, new_cursor)
                };

                // inserted text, e.g. between the quotes in `''`).
                let at_end = final_cursor as usize == final_text.len();
                ui.set_editor_text(final_text.clone().into());
                ui.set_editor_cursor_target(final_cursor);
                // Re-trigger only when the cursor is at end of inserted text AND the text
                // does not end at a syntactically terminal expression (IS NULL, TRUE, FALSE,
                // a string/numeric literal, ASC/DESC).  Terminal positions use a virtual
                // trailing space so the parser sees the next context without polluting the
                // editor with a stale space the user would have to delete before typing `;`.
                if cursor_offset_val == 0 && at_end && !is_terminal_expression(&final_text) {
                    let trigger_sql = format!("{} ", final_text);
                    ui.invoke_trigger_completion(trigger_sql.into(), final_cursor + 1);
                }
            });
        },
    );
}

pub(super) fn handle_completion_ready(
    items: Vec<wf_completion::CompletionItem>,
    ww: slint::Weak<crate::AppWindow>,
) {
    // Build Vec<CompletionRow> outside invoke_from_event_loop (Vec is Send, Rc is not).
    let rows: Vec<crate::CompletionRow> = items
        .iter()
        .map(|item| crate::CompletionRow {
            label: item.label.clone().into(),
            kind: completion_kind_label(&item.kind).into(),
            detail: item.detail.clone().unwrap_or_default().into(),
            insert_text: item.insert_text.clone().into(),
            cursor_offset: item.cursor_offset,
            table_name: item.table_name.clone().unwrap_or_default().into(),
        })
        .collect();
    // clone required: invoke_from_event_loop closure must be 'static
    let _ = slint::invoke_from_event_loop(move || {
        with_ui(&ww, move |ui| {
            if rows.is_empty() {
                ui.set_completion_visible(false);
            } else {
                let model = Rc::new(slint::VecModel::from(rows));
                ui.set_completion_items(model.into());
                ui.set_completion_selected(0);
                ui.set_completion_visible(true);
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── find_prefix_start ─────────────────────────────────────────────────────

    #[test]
    fn find_prefix_start_should_return_word_start_before_cursor() {
        assert_eq!(find_prefix_start("SELECT sel", 10), 7);
    }

    #[test]
    fn find_prefix_start_should_return_cursor_when_at_space() {
        assert_eq!(find_prefix_start("SELECT ", 7), 7);
    }

    #[test]
    fn find_prefix_start_should_return_after_dot_for_qualified_name() {
        assert_eq!(find_prefix_start("u.em", 4), 2);
    }

    #[test]
    fn find_prefix_start_should_return_cursor_when_no_prefix() {
        assert_eq!(find_prefix_start("SELECT * FROM ", 14), 14);
    }

    #[test]
    fn find_prefix_start_should_return_cursor_after_semicolon() {
        // Backspace removes trailing space: cursor lands right after `;` — no prefix.
        let sql = "select name from users;";
        assert_eq!(find_prefix_start(sql, sql.len()), sql.len());
    }

    #[test]
    fn find_prefix_start_should_return_cursor_after_comma() {
        // Cursor right after comma — no word prefix, popup should close.
        assert_eq!(find_prefix_start("SELECT id,", 10), 10);
    }

    #[test]
    fn find_prefix_start_should_return_word_start_when_prefix_non_empty() {
        // Backspace shrinks "wher" → "whe" — prefix is still non-empty, popup stays.
        assert_eq!(find_prefix_start("SELECT * FROM users WHERE whe", 29), 26);
    }

    // ── sql_has_from ──────────────────────────────────────────────────────────

    #[test]
    fn sql_has_from_should_return_true_when_from_present() {
        assert!(sql_has_from("SELECT id FROM users"));
    }

    #[test]
    fn sql_has_from_should_return_false_when_no_from() {
        assert!(!sql_has_from("SELECT id, name"));
    }

    #[test]
    fn sql_has_from_should_return_true_for_multiline_from() {
        assert!(sql_has_from("SELECT id\nFROM users"));
    }

    #[test]
    fn sql_has_from_should_return_false_for_from_in_column_name() {
        // "from" inside a word like "transform" should not match
        assert!(!sql_has_from("SELECT transform_id"));
    }

    // ── is_terminal_expression ────────────────────────────────────────────────

    #[test]
    fn is_terminal_expression_should_return_true_for_is_not_null() {
        assert!(is_terminal_expression(
            "SELECT name FROM users WHERE deleted_at IS NOT NULL"
        ));
    }

    #[test]
    fn is_terminal_expression_should_return_true_for_is_null() {
        assert!(is_terminal_expression("WHERE col IS NULL"));
    }

    #[test]
    fn is_terminal_expression_should_return_true_for_true_keyword() {
        assert!(is_terminal_expression("WHERE active = TRUE"));
    }

    #[test]
    fn is_terminal_expression_should_return_true_for_false_keyword() {
        assert!(is_terminal_expression("WHERE active = FALSE"));
    }

    #[test]
    fn is_terminal_expression_should_return_true_for_asc_keyword() {
        assert!(is_terminal_expression("ORDER BY id ASC"));
    }

    #[test]
    fn is_terminal_expression_should_return_true_for_desc_keyword() {
        assert!(is_terminal_expression("ORDER BY id DESC"));
    }

    #[test]
    fn is_terminal_expression_should_return_true_for_string_literal() {
        assert!(is_terminal_expression("WHERE name = 'alice'"));
    }

    #[test]
    fn is_terminal_expression_should_return_true_for_integer_literal() {
        assert!(is_terminal_expression("WHERE id = 5"));
    }

    #[test]
    fn is_terminal_expression_should_return_true_for_limit_clause() {
        assert!(is_terminal_expression("LIMIT 10"));
    }

    #[test]
    fn is_terminal_expression_should_return_false_after_where_keyword() {
        assert!(!is_terminal_expression("FROM users WHERE"));
    }

    #[test]
    fn is_terminal_expression_should_return_false_for_table_name_position() {
        assert!(!is_terminal_expression("SELECT * FROM users"));
    }

    #[test]
    fn is_terminal_expression_should_return_false_for_column_name_position() {
        assert!(!is_terminal_expression("SELECT id"));
    }

    #[test]
    fn is_terminal_expression_should_not_match_null_suffix_in_word() {
        // "nullify" last word is "NULLIFY" — not the keyword "NULL"
        assert!(!is_terminal_expression("WHERE nullify"));
    }

    #[test]
    fn is_terminal_expression_should_not_match_null_within_identifier() {
        // "is_not_null_col" is a column name, not the keyword NULL
        assert!(!is_terminal_expression("SELECT is_not_null_col"));
    }
}

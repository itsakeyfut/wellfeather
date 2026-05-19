use std::collections::HashMap;

use uuid::Uuid;

use crate::app::session::TabSessionEntry;

// ── Per-tab text undo/redo stack ─────────────────────────────────────────────

const UNDO_MAX: usize = 100;

pub struct TextUndoStack {
    undo: Vec<String>,
    redo: Vec<String>,
}

impl TextUndoStack {
    fn new() -> Self {
        Self {
            undo: vec![],
            redo: vec![],
        }
    }

    /// Push a snapshot of the text *before* a change. Clears redo. Deduplicates top.
    pub fn push(&mut self, text: String) {
        if self.undo.last().map(|s| s == &text).unwrap_or(false) {
            return;
        }
        self.undo.push(text);
        if self.undo.len() > UNDO_MAX {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Undo: restore previous state. `current` is pushed to redo.
    pub fn undo(&mut self, current: String) -> Option<String> {
        let prev = self.undo.pop()?;
        self.redo.push(current);
        if self.redo.len() > UNDO_MAX {
            self.redo.remove(0);
        }
        Some(prev)
    }

    /// Redo: restore next state. `current` is pushed to undo.
    pub fn redo(&mut self, current: String) -> Option<String> {
        let next = self.redo.pop()?;
        self.undo.push(current);
        if self.undo.len() > UNDO_MAX {
            self.undo.remove(0);
        }
        Some(next)
    }
}

// ── TabsState ─────────────────────────────────────────────────────────────────

pub struct TabsState {
    pub tabs: Vec<TabEntry>,
    pub active_index: usize,
    counter: usize,
    pub undo_stacks: HashMap<String, TextUndoStack>,
}

#[derive(Clone)]
pub struct TabEntry {
    pub id: String,
    pub title: String,
    pub kind: TabKind,
}

#[derive(Clone)]
pub enum TabKind {
    SqlEditor {
        query_text: String,
    },
    TableView {
        conn_id: String,
        table_name: String,
        sub_tab: usize,
    },
}

impl TabsState {
    pub fn new() -> Self {
        Self {
            tabs: vec![TabEntry {
                id: Uuid::new_v4().to_string(),
                title: "Query 1".to_string(),
                kind: TabKind::SqlEditor {
                    query_text: String::new(),
                },
            }],
            active_index: 0,
            counter: 1,
            undo_stacks: HashMap::new(),
        }
    }

    /// Restore from persisted session entries (SqlEditor tabs only).
    pub fn from_session(active_index: usize, entries: Vec<TabSessionEntry>) -> Self {
        let counter = entries.len();
        let tabs: Vec<TabEntry> = entries
            .into_iter()
            .map(|e| TabEntry {
                id: e.id,
                title: e.title,
                kind: TabKind::SqlEditor {
                    query_text: e.query_text,
                },
            })
            .collect();
        let tabs = if tabs.is_empty() {
            vec![TabEntry {
                id: Uuid::new_v4().to_string(),
                title: "Query 1".to_string(),
                kind: TabKind::SqlEditor {
                    query_text: String::new(),
                },
            }]
        } else {
            tabs
        };
        let active_index = active_index.min(tabs.len() - 1);
        Self {
            tabs,
            active_index,
            counter,
            undo_stacks: HashMap::new(),
        }
    }

    /// Save the current editor text into the active SqlEditor tab.
    pub fn save_current_text(&mut self, text: &str) {
        if let Some(tab) = self.tabs.get_mut(self.active_index)
            && let TabKind::SqlEditor { query_text } = &mut tab.kind
        {
            *query_text = text.to_string();
        }
    }

    /// Add a new SQL Editor tab. Returns `(id, new_index)`.
    pub fn add_sql_editor(&mut self) -> (String, usize) {
        self.counter += 1;
        let id = Uuid::new_v4().to_string();
        let title = format!("Query {}", self.counter);
        self.tabs.push(TabEntry {
            id: id.clone(),
            title,
            kind: TabKind::SqlEditor {
                query_text: String::new(),
            },
        });
        let idx = self.tabs.len() - 1;
        self.active_index = idx;
        (id, idx)
    }

    /// Open or focus a Table View tab. Returns `(id, index, is_new)`.
    pub fn open_table_view(&mut self, conn_id: &str, table_name: &str) -> (String, usize, bool) {
        if let Some(idx) = self.find_table_view(conn_id, table_name) {
            let id = self.tabs[idx].id.clone();
            self.active_index = idx;
            return (id, idx, false);
        }
        let id = Uuid::new_v4().to_string();
        self.tabs.push(TabEntry {
            id: id.clone(),
            title: table_name.to_string(),
            kind: TabKind::TableView {
                conn_id: conn_id.to_string(),
                table_name: table_name.to_string(),
                sub_tab: 0,
            },
        });
        let idx = self.tabs.len() - 1;
        self.active_index = idx;
        (id, idx, true)
    }

    pub fn find_table_view(&self, conn_id: &str, table_name: &str) -> Option<usize> {
        self.tabs.iter().position(|t| {
            matches!(&t.kind, TabKind::TableView { conn_id: c, table_name: n, .. } if c == conn_id && n == table_name)
        })
    }

    /// Close tab at `index`. Returns `false` if the last SqlEditor tab would be closed.
    pub fn close(&mut self, index: usize) -> bool {
        if index >= self.tabs.len() {
            return false;
        }
        let sql_count = self
            .tabs
            .iter()
            .filter(|t| matches!(t.kind, TabKind::SqlEditor { .. }))
            .count();
        if matches!(self.tabs[index].kind, TabKind::SqlEditor { .. }) && sql_count <= 1 {
            return false;
        }
        let tab_id = self.tabs[index].id.clone();
        self.undo_stacks.remove(&tab_id);
        self.tabs.remove(index);
        if self.active_index >= self.tabs.len() {
            self.active_index = self.tabs.len().saturating_sub(1);
        }
        true
    }

    pub fn push_undo_snapshot(&mut self, tab_id: &str, text: String) {
        self.undo_stacks
            .entry(tab_id.to_string())
            .or_insert_with(TextUndoStack::new)
            .push(text);
    }

    pub fn text_undo(&mut self, tab_id: &str, current: String) -> Option<String> {
        self.undo_stacks.get_mut(tab_id)?.undo(current)
    }

    pub fn text_redo(&mut self, tab_id: &str, current: String) -> Option<String> {
        self.undo_stacks.get_mut(tab_id)?.redo(current)
    }

    pub fn set_active(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.active_index = index;
        }
    }

    pub fn active_tab(&self) -> Option<&TabEntry> {
        self.tabs.get(self.active_index)
    }

    /// Save the sub-tab index for the active TableView tab.
    pub fn save_tv_sub_tab(&mut self, sub_tab: usize) {
        if let Some(tab) = self.tabs.get_mut(self.active_index)
            && let TabKind::TableView {
                sub_tab: ref mut st,
                ..
            } = tab.kind
        {
            *st = sub_tab;
        }
    }

    /// Return the sub-tab index stored for the active TableView tab.
    pub fn active_tv_sub_tab(&self) -> usize {
        if let Some(tab) = self.tabs.get(self.active_index)
            && let TabKind::TableView { sub_tab, .. } = tab.kind
        {
            sub_tab
        } else {
            0
        }
    }

    /// Extract session-persistable data (SqlEditor tabs only).
    /// Returns `(active_sql_index, entries)`.
    pub fn session_entries(&self) -> (usize, Vec<TabSessionEntry>) {
        let sql_tabs: Vec<(usize, &TabEntry)> = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(_, t)| matches!(t.kind, TabKind::SqlEditor { .. }))
            .collect();

        let active_sql_idx = sql_tabs
            .iter()
            .position(|(i, _)| *i == self.active_index)
            .unwrap_or(0);

        let entries = sql_tabs
            .into_iter()
            .map(|(_, t)| {
                let qt = if let TabKind::SqlEditor { query_text } = &t.kind {
                    query_text.clone()
                } else {
                    String::new()
                };
                TabSessionEntry {
                    id: t.id.clone(),
                    title: t.title.clone(),
                    query_text: qt,
                }
            })
            .collect();

        (active_sql_idx, entries)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_undo_stack_should_push_and_undo() {
        let mut s = TextUndoStack::new();
        s.push("hello".to_string());
        let restored = s.undo("hello world".to_string());
        assert_eq!(restored, Some("hello".to_string()));
    }

    #[test]
    fn text_undo_stack_should_not_push_duplicate() {
        let mut s = TextUndoStack::new();
        s.push("hello".to_string());
        s.push("hello".to_string());
        assert_eq!(s.undo.len(), 1);
    }

    #[test]
    fn text_undo_stack_should_clear_redo_on_push() {
        let mut s = TextUndoStack::new();
        s.push("a".to_string());
        s.undo("b".to_string());
        assert_eq!(s.redo.len(), 1);
        s.push("c".to_string());
        assert_eq!(s.redo.len(), 0);
    }

    #[test]
    fn text_undo_stack_should_redo_after_undo() {
        let mut s = TextUndoStack::new();
        s.push("before".to_string());
        let _prev = s.undo("after".to_string());
        let redone = s.redo("before".to_string());
        assert_eq!(redone, Some("after".to_string()));
    }

    #[test]
    fn text_undo_stack_should_cap_at_max_size() {
        let mut s = TextUndoStack::new();
        for i in 0..=UNDO_MAX {
            s.push(format!("text{i}"));
        }
        assert_eq!(s.undo.len(), UNDO_MAX);
    }
}

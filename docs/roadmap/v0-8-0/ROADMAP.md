# v0.8.0 — Editor Enhancement (Syntax Highlighting + Multi-tab)

> **Theme**: Make the editor richer with syntax highlighting and multi-tab support.
> **Prerequisite**: v0.7.0

---

## Goal

Implement syntax highlighting and multi-tab support so users can write multiple queries
in parallel, each with its own result state.

---

## Exit Criteria

- [ ] SQL keywords, string literals, and comments are color-coded in the editor
- [ ] Multiple editor tabs can be opened, each managing an independent query and result
- [ ] Tabs share the active connection (one connection per app)
- [ ] Tabs can be added (`Ctrl+T`), closed (`Ctrl+W`), and navigated (`Ctrl+Tab`)
- [ ] Tab state (query content + file path) is restored on restart
- [ ] Result panel supports multiple result tabs for multi-statement queries

---

## Key Risks

- **Syntax highlighting in Slint**: Slint's `TextInput` does not natively support per-token styling. A custom rendering approach may be required. Complete the investigation spike (#84) before starting implementation
- Multi-tab requires a state management redesign: `TabEntry` per tab, `active-tab-index`, independent query text and result per tab

---

## Task List

See `docs/roadmap/tasks/v0-8-0.md` for details.

| Task ID | Title | Issue |
|---------|-------|-------|
| — | Slint syntax highlighting: technical investigation spike | #84 |
| T111 | SQL syntax highlighting in editor | #59 |
| T112 | Multi-tab editor UI | #60 |
| T113 | Per-tab state management (TabState in AppState) | #61 |
| — | Tab system foundation (TabEntry struct, UiState binding) | #120 |
| — | Tab data model: TabEntry + UiState.tabs / active-tab-index | #125 |
| — | Tab bar UI: scrollable bar with active highlight, dirty *, close × | #126 |
| — | Tab content binding: sync editor text to active tab | #127 |
| — | Tab keyboard shortcuts: Ctrl+T / Ctrl+W / Ctrl+Tab | #128 |
| — | Session restore: persist tab query content + file paths | #129 |
| — | Tab drag-and-drop reordering | #136 |
| — | Result panel tabs for multi-statement query results | #186 |

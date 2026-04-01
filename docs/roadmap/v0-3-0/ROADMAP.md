# v0.3.0 — Query Execution Core

> **Theme**: Write SQL, run it, see results. The heart of wellfeather's core value.
> **Prerequisite**: v0.2.0

---

## Goal

Make it possible to write a query in the SQL editor, execute it, and view the results.
This is wellfeather's most important user experience — this version is where it becomes a usable tool.

---

## Exit Criteria

- [ ] Multi-line SQL can be typed in the editor (with line numbers)
- [ ] Ctrl+Enter executes the SQL statement at the cursor position
- [ ] Ctrl+Shift+Enter executes the full editor contents
- [ ] Shift+Enter executes the selected range
- [ ] The result table shows column headers and row data
- [ ] A running query can be cancelled with Esc / Cancel
- [ ] Errors are displayed inline in the result area
- [ ] The status bar shows execution time (ms) and row count
- [ ] Each query execution is recorded in `history.db`
- [ ] The UI remains responsive during query execution (non-blocking)

---

## Scope

| Category | Content |
|----------|---------|
| SQL editor | editor.slint — multiline text area with line numbers |
| Keyboard shortcuts | Ctrl+Enter / Ctrl+Shift+Enter / Shift+Enter / Esc |
| DB execution layer | db/drivers/ — execute implementation (all DBs), CancellationToken support |
| DbService | execute / execute_with_cancel |
| Controller | Command::RunQuery / RunSelection / RunAll / CancelQuery handling |
| Async flow | tokio::spawn + invoke_from_event_loop for UI updates |
| Result table | result_table.slint — basic table (columns + rows, VecModel) |
| Error display | Inline error rendering in the result area |
| Status bar | Execution time and row count updates |
| Query history | history/service.rs — save execution log to SQLite |

---

## Out of Scope

- Virtual scroll (v0.5.0)
- NULL badges and column sort (v0.5.0)
- SQL completion and formatter (v0.6.0)
- Sidebar metadata display (v0.4.0)

---

## Key Risks

- **`invoke_from_event_loop`**: `Rc` cannot be used inside the closure. Create `VecModel` inside the closure (see reference-patterns.md)
- Slint's `TextInput` may have limitations for custom key event handling in multiline mode — verify Ctrl+Enter binding behavior carefully
- SQL statement extraction at cursor position (`;`-delimited) is factored into query/analyzer.rs
- `tokio::select!`-based cancellation timing depends on the DB driver — sqlx may not support `cancel_token` directly and may require dropping the connection instead

---

## Task List

See `docs/roadmap/tasks/v0-3-0.md` for details.

| Task ID | Title | Issue |
|---------|-------|-------|
| T031 | editor.slint — SQL editor with line numbers | #19 |
| T032 | query/analyzer.rs — extract SQL statement at cursor position | #20 |
| T033 | db/drivers/ — execute implementation (SQLite, PG, MySQL) | #21 |
| T034 | db/service.rs — execute + execute_with_cancel | #22 |
| T035 | app/controller.rs — RunQuery / Cancel command handling | #23 |
| T036 | result_table.slint — basic query result table | #24 |
| T037 | Inline SQL error display in result area | #25 |
| T038 | status_bar.slint — execution time and row count display | #26 |
| T039 | history/service.rs — HistoryService with SQLite persistence | #27 |

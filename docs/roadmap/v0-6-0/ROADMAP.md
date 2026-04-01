# v0.6.0 — SQL Writing Experience

> **Theme**: Bring the SQL writing experience close to DataGrip quality — lightweight and fast.
> **Prerequisite**: v0.5.0

---

## Goal

Implement SQL completion and a formatter to create a "easy to write, easy to read" query experience.
Target completion quality better than DBeaver / TablePlus, and a formatter on par with DataGrip,
both implemented lightweight and fast.

---

## Exit Criteria

- [ ] Completion candidates appear automatically 300ms after typing stops
- [ ] Ctrl+Space triggers completion manually
- [ ] SQL keywords (SELECT, WHERE, JOIN, etc.) are completed
- [ ] Table names from the connected DB are completed
- [ ] Column names are completed based on the FROM clause context
- [ ] Candidates can be selected with ↑↓/Enter and dismissed with Esc
- [ ] Ctrl+Shift+F formats the SQL (indentation + keyword uppercasing)
- [ ] Both completion and formatter respond in < 100ms

---

## Scope

| Category | Content |
|----------|---------|
| Parser | completion/parser.rs — cursor-position context analysis |
| Completion engine | completion/engine.rs — keyword / table / column candidate generation |
| CompletionService | completion/service.rs — 300ms debounce + Ctrl+Space trigger |
| Completion UI | Slint popup (candidate list with keyboard selection) |
| SQL formatter | query/formatter.rs — lightweight formatting logic |
| Shortcut | Ctrl+Shift+F triggers formatter |

---

## Out of Scope

- LSP (sql-language-server) integration (Phase 3)
- Type-inference-based completion (Phase 3)
- Syntax highlighting (v1.1.0)
- Alias-aware column completion (future)

---

## Key Risks

- **Completion popup in Slint**: Overlaying a popup at the cursor position using `PopupWindow` is possible but requires care for accurate cursor-relative positioning
- Slint's `TextInput` may have a limited API for retrieving cursor position (character index) — verify before implementation
- Decide whether to use an external crate (`sqlformat` etc.) or a custom formatter implementation:
  - Prefer `sqlformat` (pure Rust, lightweight) for speed
- Debounce for completion uses `slint::Timer::SingleShot` (see reference-patterns.md)

---

## Task List

See `docs/roadmap/tasks/v0-6-0.md` for details.

| Task ID | Title | Issue |
|---------|-------|-------|
| T061 | completion/parser.rs — cursor-position context analysis | #40 |
| T062 | completion/engine.rs — completion candidate generation | #41 |
| T063 | completion/service.rs — CompletionService with 300ms debounce | #42 |
| T064 | Completion popup UI in editor | #43 |
| T065 | query/formatter.rs — SQL formatter (Ctrl+Shift+F) | #44 |

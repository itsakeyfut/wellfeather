# v0.10.0 — Editor Enhancement and Data Editing

> **Theme**: Write queries faster and edit data without leaving the app.
> **Prerequisite**: v0.9.0

---

## Goal

Add multi-tab query editing, code snippets, parameterized queries, and inline cell editing
so wellfeather can replace a dedicated SQL IDE for day-to-day development work.
SOCKS5 proxy support is included here as the natural extension of the SSH Tunnel tab
introduced in v0.9.0.

---

## Exit Criteria

- [ ] Multiple editor tabs open simultaneously, each with independent SQL text and result state
- [ ] Tab content (SQL text, tab name, order) persists across restarts via `session.toml`
- [ ] Code snippets saved to `snippets.toml`; insertable via sidebar double-click or `Ctrl+Shift+S` palette
- [ ] `:name` placeholders auto-detected; variable input dialog shown before execution
- [ ] JSON/XML cells auto-detected and shown as a collapsible tree in the bottom pane
- [ ] Form view available as a toolbar toggle on the result table
- [ ] Cell double-click enters edit mode; uncommitted changes shown with a colored background
- [ ] NULL assignment, row-add, and row-delete all work with PK-based UPDATE/INSERT/DELETE
- [ ] SOCKS5 proxy config available in the SSH Tunnel tab via a tunnel-type radio button

---

## Key Risks

- **Multi-tab state serialization** — `TabEntry` ordering and dirty state must survive app crashes without data loss
- **Inline editing** — UPDATE generation requires reliable PK detection; read-only result sets (multi-table JOIN, no PK) must show a clear disabled state rather than silently failing
- **JSON/XML tree** — parsing is best-effort; malformed values must fall back to plain text gracefully with no UI crash

---

## Task List

See `docs/roadmap/tasks/v0-10-0.md` for details.

| Task ID | Title |
|---------|-------|
| T101 | Multi-tab editor: TabEntry state, tab bar UI, Ctrl+T/W/Tab, session persistence |
| T102 | Code snippets: snippets.toml, sidebar section, Ctrl+Shift+S palette |
| T103 | Parameterized queries: :name detection, variable input dialog, editor highlight |
| T104 | JSON/XML tree viewer in bottom pane with path-copy and value-copy |
| T105 | Form view: record-by-record display with Alt+↑/↓ navigation |
| T106 | Inline cell editing: PK-based UPDATE/INSERT/DELETE with commit/rollback toolbar |
| T107 | SOCKS5 proxy: tunnel-type radio in SSH Tunnel tab |

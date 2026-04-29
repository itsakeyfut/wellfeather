# v0.14.0 — File Operations, Settings UI, and Table Browser

> **Theme**: Persist work across sessions and browse table data directly.
> **Prerequisite**: v0.8.0

---

## Goal

Add SQL file save/load, a settings UI, and an interactive table browser tab
so users can manage their work outside of just running queries.

---

## Exit Criteria

- [ ] SQL files can be opened (`Ctrl+O`) and saved (`Ctrl+S` / `Ctrl+Shift+S`) per tab
- [ ] Dirty tabs show a `*` indicator and prompt for confirmation before close/quit
- [ ] Recent files (last 10) are listed and accessible from the File menu
- [ ] Double-clicking a table in the sidebar opens a table browser tab with paginated data
- [ ] The table browser toolbar includes a Refresh button and a LIMIT selector
- [ ] Font, theme, timeout, slow query threshold, and language can be changed from a settings panel without editing `config.toml` by hand

---

## Task List

See `docs/roadmap/tasks/v0-14-0.md` for details.

| Task ID | Title | Issue |
|---------|-------|-------|
| — | rfd file dialog integration (Command::OpenFile / SaveFile / SaveFileAs) | #132 |
| — | File path + dirty state in TabEntry: Ctrl+S / Ctrl+Shift+S / Ctrl+O wiring | #133 |
| — | Dirty close/quit confirmation dialog: Save / Discard / Cancel | #134 |
| — | Recent files list: 10-item history persisted in config.toml | #135 |
| — | SQL file save/load: persist query tabs as .sql files | #122 |
| — | Command::BrowseTable + Event::TableBrowseResult: sidebar double-click backend | #130 |
| — | Table browser tab UI: toolbar (▶ Refresh, LIMIT selector) + tab deduplication | #131 |
| — | Table browser tab: double-click table opens data viewer tab | #121 |
| — | Settings screen UI: interactive configuration panel | #85 |

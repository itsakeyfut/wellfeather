# v0.8.0 — Quick Wins (DDL Viewer, Safe DML, Find/Replace, and More)

> **Theme**: High-value, low-cost improvements that sharpen the everyday SQL experience.
> **Prerequisite**: v0.7.0

---

## Goal

Deliver eight standalone improvements that each solve a real pain point with minimal
cross-feature coupling. This milestone establishes patterns (DDL viewer, bottom-pane reuse,
sidebar sections) that later milestones will extend.

---

## Exit Criteria

- [ ] Sidebar object single-click shows the DDL CREATE statement in the bottom pane
- [ ] Safe DML mode warns before executing `UPDATE`/`DELETE` without a `WHERE` clause
- [ ] Read-only mode blocks writes and shows a lock icon on the connection
- [ ] `Ctrl+F` opens an inline find bar; `Ctrl+H` opens find + replace
- [ ] Bookmarks are saved to `bookmarks.toml` and accessible from a sidebar section
- [ ] Connection color dots appear in the sidebar and the status bar active-connection display
- [ ] Result table supports INSERT SQL export via File → Export → Insert SQL
- [ ] `Ctrl+P` opens a floating metadata global search palette

---

## Key Risks

- **DDL viewer** — PostgreSQL has no single `pg_get_tabledef` function; DDL must be
  reconstructed by combining `information_schema` and `pg_catalog` queries
- **Find/replace in Slint** — `TextInput` selection-and-replace requires
  `set-selection-offsets`; verify API availability before implementation

---

## Task List

See `docs/roadmap/tasks/v0-8-0.md` for details.

| Task ID | Title |
|---------|-------|
| T081 | DDL viewer: bottom-pane DDL display on sidebar click |
| T082 | Safe DML mode: WHERE-less UPDATE/DELETE detection and confirmation dialog |
| T083 | Read-only connection mode: write guard and UI indicator |
| T084 | Editor find / find-replace floating bar (Ctrl+F / Ctrl+H) |
| T085 | Query bookmarks: bookmarks.toml persistence and sidebar section |
| T086 | Connection color coding: color dot in sidebar and status bar |
| T087 | INSERT SQL export from result table |
| T088 | Metadata global search palette (Ctrl+P) |

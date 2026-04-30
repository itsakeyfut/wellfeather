# v0.11.0 — Schema Tools and Query Analysis

> **Theme**: Understand and change database structure without leaving the app.
> **Prerequisite**: v0.10.0

---

## Goal

Give users full visibility into their schema (ER diagram, diff, DDL editor) and tools to
analyze query performance (EXPLAIN visualizer, visual filter builder) so they rarely need
to switch to another tool. The ER diagram is rendered natively via Slint Canvas API —
no embedded browser or WebView dependency.

---

## Exit Criteria

- [ ] Visual filter builder panel applies client-side filters in real time without re-querying the DB
- [ ] EXPLAIN (ANALYZE) runs and displays a cost-annotated node tree; Seq Scan nodes highlighted red
- [ ] Table structure editor: columns and indexes viewable and editable via a two-tab dialog
- [ ] ALTER TABLE / CREATE INDEX / DROP INDEX DDL shown as preview before applying
- [ ] ER diagram renders FK relationships via Slint Canvas; nodes are draggable; zoom works
- [ ] ER diagram PNG/SVG export works
- [ ] Schema diff shows Added/Removed/Modified objects side-by-side between two live connections
- [ ] Migration script generated from diff and inserted into the editor

---

## Key Risks

- **ER diagram with Slint Canvas** — Fruchterman–Reingold layout must be implemented in Rust; test with >50-table schemas for performance (target: layout completes in < 1 second)
- **EXPLAIN JSON parsing** — PostgreSQL, MySQL, and SQLite each produce different output formats; a unified `ExplainNode` tree abstraction is required
- **Schema diff** — DDL reconstruction from `information_schema` must be consistent across PG, MySQL, and SQLite to produce comparable output

---

## Task List

See `docs/roadmap/tasks/v0-11-0.md` for details.

| Task ID | Title |
|---------|-------|
| T111 | Visual filter builder: real-time client-side filter panel with AND/OR logic |
| T112 | EXPLAIN visualizer: per-DB execution, JSON parse to ExplainNode, tree UI |
| T113 | Table structure editor + index management: two-tab dialog with DDL preview |
| T114 | ER diagram: Slint Canvas rendering, Fruchterman–Reingold layout, PNG/SVG export |
| T115 | Schema diff: DDL comparison between two connections, migration script generation |

# v0.4.0 — Schema Browser

> **Theme**: Understand the DB structure visually and start queries quickly from table names.
> **Prerequisite**: v0.3.0

---

## Goal

Display a schema tree of the connected database in the sidebar.
Double-clicking a table name inserts `SELECT * FROM <table>` into the editor,
enabling an instant query-and-explore workflow.

---

## Exit Criteria

- [ ] After connecting, the sidebar shows a schema tree
  - Four categories: Tables / Views / Stored Procedures / Indexes
- [ ] Each node can be collapsed and expanded
- [ ] Double-clicking a table name inserts `SELECT * FROM <table>` into the editor
- [ ] Metadata is fetched asynchronously in the background on connect (UI stays responsive)
- [ ] Fetched metadata is stored in MetadataCache (memory + SQLite flush)
- [ ] Alt+↑/↓/←/→ moves focus between the sidebar, editor, and result table

---

## Scope

| Category | Content |
|----------|---------|
| Metadata fetch | db/drivers/ — fetch_metadata implementation (all DBs) |
| MetadataCache | completion/cache.rs — memory cache with SQLite flush |
| Controller | Trigger async fetch_metadata after connect; send Event::MetadataLoaded |
| Sidebar UI | sidebar.slint — collapsible tree structure |
| Table double-click | Insert `SELECT * FROM <table>` from sidebar into editor |
| Pane navigation | Alt+Arrow focus movement |

---

## Out of Scope

- DDL display and column detail view (future)
- Manual metadata refresh button (future)
- Detailed index and stored procedure views (future)

---

## Key Risks

- **Slint tree view**: Slint has no built-in tree view component — implement using nested `ListView` with conditional indentation
- Each DB has different metadata query SQL; implement separately per driver:
  - PostgreSQL: `information_schema.tables / columns`
  - MySQL: `information_schema.tables / columns`
  - SQLite: `sqlite_master` (tables only; views/procs/indexes are limited)
- Watch for sidebar rendering performance with large schemas (hundreds of tables)
- Verify that Alt+Arrow does not conflict with OS or Slint default key bindings

---

## Task List

See `docs/roadmap/tasks/v0-4-0.md` for details.

| Task ID | Title | Issue |
|---------|-------|-------|
| T041 | db/drivers/ — fetch_metadata implementation (SQLite, PG, MySQL) | #28 |
| T042 | completion/cache.rs — MetadataCache (memory + SQLite flush) | #29 |
| T043 | app/controller.rs — background metadata fetch on connect | #30 |
| T044 | sidebar.slint — collapsible schema tree UI | #31 |
| T045 | Table double-click inserts SELECT * FROM into editor | #32 |
| T046 | Alt+Arrow pane focus navigation | #33 |

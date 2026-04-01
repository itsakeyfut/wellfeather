# v0.2.0 — DB Connection Management

> **Theme**: Connect to a real database. Manage connection credentials securely.
> **Prerequisite**: v0.1.0

---

## Goal

Enable connecting to, disconnecting from, and switching between PostgreSQL, MySQL, and SQLite databases.
Connection credentials are persisted in TOML encrypted with AES-256-GCM and automatically restored on next launch.

---

## Exit Criteria

- [ ] Connection credentials can be added via the UI and used to connect (SQLite / PostgreSQL / MySQL)
- [ ] Multiple connections can be held simultaneously and switched between
- [ ] Connection credentials are saved to `config.toml` in encrypted form
- [ ] The previous connection is automatically restored after restarting the app
- [ ] The status bar displays the active connection name and database name
- [ ] Integration test: SQLite connection test passes

---

## Scope

| Category | Content |
|----------|---------|
| DB connection layer | db/pool.rs (DbPool enum), db/drivers/sqlite.rs, pg.rs, my.rs (connect only) |
| DbService | db/service.rs — connect / disconnect / pools HashMap management |
| Controller | app/controller.rs — Command::Connect / Disconnect handling, event loop start |
| Connection management UI | Add-connection dialog (individual fields + connection string) |
| Connection list UI | Sidebar connection list with active connection switching |
| Session | app/session.rs — save last connection, auto-connect on startup |
| Status bar | Display connection name and database name |

---

## Out of Scope

- Query execution (connection establishment only)
- Metadata fetch (v0.4.0)
- Schema tree expansion in the sidebar (v0.4.0)

---

## Key Risks

- Use `DbPool` enum dispatch instead of sqlx `AnyPool` (see architecture.md)
- Connection timeout and retry on failure are out of scope for MVP — show an error only
- MySQL / PostgreSQL integration tests require a live database; handle carefully in CI
  - Prioritize local SQLite integration tests; treat PG/MySQL tests as optional
- The encryption key (`crypto::key()`) must be the one established in v0.1.0

---

## Task List

See `docs/roadmap/tasks/v0-2-0.md` for details.

| Task ID | Title | Issue |
|---------|-------|-------|
| T021 | db/pool.rs — DbPool enum and connect function | #10 |
| T022 | db/drivers/sqlite.rs — SQLite connection implementation | #11 |
| T023 | db/drivers/pg.rs — PostgreSQL connection implementation | #12 |
| T024 | db/drivers/my.rs — MySQL connection implementation | #13 |
| T025 | db/service.rs — DbService connect/disconnect | #14 |
| T026 | app/controller.rs — Command loop + Connect/Disconnect handling | #15 |
| T027 | Connection management UI: add dialog + list/switch | #16 |
| T028 | app/session.rs — connection session save and restore | #17 |
| T029 | status_bar.slint — connection name/DB display | #18 |

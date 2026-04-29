# v0.10.0 — History UI, Slow Query Detection, and Timeout

> **Theme**: Make query history actionable and surface performance issues early.
> **Prerequisite**: v0.9.0

---

## Goal

Implement a history UI for searching and reusing past queries, alert functionality for slow queries,
and configurable query timeout so users can manage long-running operations.

---

## Exit Criteria

- [ ] Query history is viewable via the UI (list display)
- [ ] History can be searched and filtered by keyword
- [ ] Clicking a history entry inserts it into the editor
- [ ] A timeout duration can be configured; queries that exceed it are automatically cancelled
- [ ] Queries exceeding a configured threshold (e.g. 1 second) trigger a warning in the status bar

---

## Task List

See `docs/roadmap/tasks/v0-10-0.md` for details.

| Task ID | Title | Issue |
|---------|-------|-------|
| T131 | HistoryService: keyword search and connection filter | #65 |
| T132 | Query history panel UI (list, search, insert, re-run) | #66 |
| T133 | Configurable query timeout with auto-cancel | #67 |
| T134 | Slow query detection and status bar warning | #68 |

# v0.13.0 — CLI Integration and LSP Completion

> **Theme**: Support hybrid GUI + CLI workflows and deliver top-tier SQL completion.
> **Prerequisite**: v0.12.0

---

## Goal

Implement CLI-based query execution and LSP-powered high-accuracy SQL completion
to support diverse developer workflows.

---

## Exit Criteria

- [ ] `wellfeather query "SELECT * FROM users"` executes a query from the CLI
- [ ] CLI and GUI share connection config and the query engine
- [ ] LSP-based completion (e.g. sql-language-server) is operational, with fallback to the built-in completion engine when the LSP server is unavailable

---

## Task List

See `docs/roadmap/tasks/v0-13-0.md` for details.

| Task ID | Title | Issue |
|---------|-------|-------|
| T211 | CLI subcommand: wellfeather query | #75 |
| T212 | Refactor: CLI/GUI shared query engine separation | #76 |
| T213 | LSP-based SQL completion integration | #77 |

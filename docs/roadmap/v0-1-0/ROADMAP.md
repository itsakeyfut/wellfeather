# v0.1.0 — Project Foundation

> **Theme**: Build the foundation. Get the app to launch.
> **Prerequisite**: none (initial release)

---

## Goal

Establish the foundation needed to start writing code.
Set up dependency crates, core data models, configuration management, encryption, state management, and the Slint UI shell so that **the app starts and displays an empty layout**.

---

## Exit Criteria

- [ ] `cargo build --release` succeeds
- [ ] Launching the app shows an empty window with the sidebar / editor / result area / status bar layout
- [ ] Startup time < 1 second (release build)
- [ ] `cargo test` passes (unit tests for config and crypto)
- [ ] `config.toml` is generated in the OS config directory on first launch

---

## Scope

| Category | Content |
|----------|---------|
| Project setup | Cargo.toml (workspace + dependency definitions), build.rs (Slint compilation) |
| Dev toolchain | justfile recipes, xtask pre-commit hook installer |
| Directory structure | Full module skeleton as described in architecture.md |
| Configuration | config/models.rs (Config struct), config/manager.rs (TOML read/write) |
| Encryption | crypto/mod.rs (AES-256-GCM encrypt/decrypt) |
| Data models | db/models.rs (DbConnection, QueryResult, etc.), db/error.rs |
| State management | state/ (AppState, ConnectionState, QueryState, UiState) |
| Command/Event | app/command.rs, app/event.rs |
| UI shell | ui/app.slint (empty layout), main.rs (Slint + tokio startup) |

---

## Out of Scope

- Actual DB connections
- Query execution
- Functional UI components (layout skeleton only)

---

## Key Risks

- Slint's build.rs configuration must be correct from the start — changing it later makes compile errors hard to trace
- Pay attention to `slint::include_modules!()` and Slint global naming (must match generated code)
- Decide upfront where to store the AES master key:
  - MVP: store as a `.key` file in the OS config directory, generated on first launch
- The `directories` crate resolves different paths on Windows / macOS / Linux — account for this early

---

## Task List

See `docs/roadmap/tasks/v0-1-0.md` for details.

| Task ID | Title | Issue |
|---------|-------|-------|
| T011 | Cargo.toml & build.rs initial setup | #1 |
| T012 | Create directory structure and module skeletons | #2 |
| T013 | config/models.rs — configuration model definitions | #3 |
| T014 | config/manager.rs — config file load/save | #4 |
| T015 | crypto/mod.rs — AES-256-GCM encryption | #5 |
| T016 | db/models.rs + db/error.rs — DB data model definitions | #6 |
| T017 | state/ — Arc\<AppState\> shared state management | #7 |
| T018 | app/command.rs + app/event.rs — Command/Event enum definitions | #8 |
| T019 | UI shell: main.rs + app.slint basic layout | #9 |
| — | Dev toolchain setup: justfile + xtask pre-commit hooks | #83 |

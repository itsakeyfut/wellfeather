# wellfeather

A lightweight, cross-platform SQL client built with Rust and [Slint](https://slint.dev).

Supports PostgreSQL, MySQL, and SQLite with features including syntax-aware autocompletion,
query history, result export (CSV/JSON), and encrypted connection profile storage.

## Status

Early development — v0.1.0 in progress.

## Architecture

Cargo workspace: `app/` binary + five library crates.

```
wf-db          — DB pool (PG/MySQL/SQLite), DbService, schema fetch
wf-config      — config.toml load/save, AES-256-GCM password encryption
wf-query       — cursor SQL analyzer, formatter, CSV/JSON export
wf-completion  — CompletionService (debounce), MetadataCache, CompletionEngine
wf-history     — SQLite-backed QueryExecution history
      │
      └──→  app  (Slint UI + tokio runtime + AppController)
```

## Build

**Prerequisites**: Rust 1.93+, [just](https://github.com/casey/just)

```bash
just build        # debug build
just run          # run the app
just ci           # fmt-check + clippy + build + test
```

## License

TBD

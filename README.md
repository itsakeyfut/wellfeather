# wellfeather

**A lightweight, keyboard-centric SQL client for PostgreSQL, MySQL, and SQLite — built with Rust and [Slint](https://slint.dev).**

[![Release](https://img.shields.io/github/v/release/itsakeyfut/wellfeather)](https://github.com/itsakeyfut/wellfeather/releases/latest)
[![CI](https://github.com/itsakeyfut/wellfeather/actions/workflows/ci.yml/badge.svg)](https://github.com/itsakeyfut/wellfeather/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

## Overview

wellfeather is a native desktop SQL client that stays out of your way.
No Electron, no JVM — just a single ~10 MB binary that starts instantly.

### Features

- **Multi-database support** — PostgreSQL, MySQL, and SQLite from one interface
- **Keyboard-first navigation** — move between panes, rows, and cells without a mouse
- **Syntax-aware autocompletion** — keywords, table names, column names, and aliases with 300 ms debounce
- **SQL formatter** — `Ctrl+Shift+F` reformats the current statement via wf-query
- **Schema browser** — collapsible three-level tree; double-click a table to insert `SELECT * FROM` into the editor
- **Result table** — virtual scrolling, column sort, NULL badges, cell/row copy, client-side filter
- **Cell preview** — full value shown in bottom pane; essential for long text or JSON
- **Export** — CSV (with BOM) and JSON to a save dialog
- **Encrypted credentials** — AES-256-GCM encryption for stored passwords
- **Session restore** — editor text and active connection persisted across restarts
- **Dark / light theme** — toggleable at runtime
- **Localization** — English and Japanese; switch via `config.toml`
- **Query history** — every execution stored in SQLite with timing metadata

---

## Installation

Download the latest binary from the [Releases](https://github.com/itsakeyfut/wellfeather/releases/latest) page.

| Platform | Asset |
|----------|-------|
| Windows x86-64 | `wellfeather-vX.Y.Z-x86_64-windows.zip` |

Extract the zip and run `wellfeather.exe`. No installer required.

> macOS and Linux builds are planned for a future release.

### Build from source

**Prerequisites**: Rust 1.93+, [just](https://github.com/casey/just)

```bash
git clone https://github.com/itsakeyfut/wellfeather
cd wellfeather
just build        # debug build → target/debug/wellfeather.exe
just build-release  # optimised build → target/release/wellfeather.exe
just run          # build + launch
```

---

## Quick Start

### Connect to a database

1. Click **+ Add connection** in the sidebar (or press `Alt+Left` to focus the sidebar, then navigate down).
2. Enter a connection string or fill in the individual fields.
3. Press **Test Connection** to verify, then **Add**.
4. Click the connection name in the sidebar to connect.

### Run a query

| Action | Shortcut |
|--------|----------|
| Execute statement at cursor | `Ctrl+Enter` |
| Execute entire editor | `Ctrl+Shift+Enter` |
| Cancel running query | `Esc` |
| Format SQL | `Ctrl+Shift+F` |

### Navigate the UI

| Action | Shortcut |
|--------|----------|
| Focus sidebar | `Alt+Left` |
| Focus editor | `Alt+Up` |
| Focus result table | `Alt+Down` |
| Move between rows | `Up` / `Down` |
| Move between cells | `Left` / `Right` |
| Copy cell value | `Ctrl+C` (cell mode) |

### Change language

Edit `config.toml` (created on first launch in the app data directory):

```toml
[ui]
language = "ja"   # "en" (default) or "ja"
```

---

## Workspace Structure

```
wellfeather/
├── app/                  — binary crate: Slint UI, tokio runtime, AppController
└── crates/
    ├── wf-db/            — DB pool (PG/MySQL/SQLite), DbService, query execution
    ├── wf-config/        — config.toml load/save, AES-256-GCM encryption
    ├── wf-query/         — cursor SQL analyzer, formatter, CSV/JSON export
    ├── wf-completion/    — CompletionService (debounce), MetadataCache, CompletionEngine
    └── wf-history/       — SQLite-backed QueryExecution history
```

`app/` is the only crate that depends on Slint. All library crates are pure Rust with no UI dependency.

---

## Platform Support

| Platform | Status |
|----------|--------|
| Windows x86-64 | Supported |
| macOS | Planned |
| Linux | Planned |

---

## MSRV

Rust **1.93** (edition 2024). Tracked via `rust-version` in `Cargo.toml`.

---

## License

Apache License 2.0 — see [LICENSE](LICENSE).

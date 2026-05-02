# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.8.0] - 2026-05-02

### v0.8.0 - Multi-tab Interface

- Multi-tab UI: SQL Editor tabs and Table View tabs (open, switch, close, Ctrl+T / Ctrl+W)
- Table View tab: data grid, DDL viewer, column list, and per-tab page-size selector
- DDL viewer: sidebar single-click shows CREATE statement in bottom pane with copy button
- Safe DML mode: WHERE-less UPDATE/DELETE triggers a confirmation dialog before execution
- Read-only connection mode: write statements blocked at controller level; lock icon in sidebar and status bar
- SVG icon set applied across the full UI (sidebar, buttons, status bar)
- Delete connection from the DB manager modal
- Platform system font applied at startup (Segoe UI on Windows, Helvetica Neue on macOS) to fix fontique rendering
- Layout / Typography / Icons design tokens centralised in theme.slint
- Connection storage migrated from config.toml to SQLite-backed ConnectionRepository
- All saved connections visible in sidebar and DB manager even when no DB is running
- Auto-connect on startup driven by last_used_at (most recently connected) instead of last_connection_id
- Tab session persistence: open tabs and active tab index saved and restored across restarts

## [0.7.0] - 2026-04-29

First public release. Covers milestones v0.1.0 through v0.7.0.

### v0.1.0 - Project Foundation

- Cargo workspace with app + five library crates (wf-db, wf-config, wf-query, wf-completion, wf-history)
- Configuration model and file load/save (config.toml)
- AES-256-GCM password encryption for stored credentials
- Arc<AppState> shared state management
- Command/Event enum pattern for UI/controller communication
- UI shell with four-pane layout (sidebar, editor, result, status bar)
- justfile + xtask pre-commit hooks

### v0.2.0 - DB Connection Management

- DbPool enum with SQLite, PostgreSQL, and MySQL drivers
- DbService connect/disconnect with error mapping
- AppController command loop with Connect/Disconnect handling
- Connection management UI: add dialog, sidebar list, one-click switching
- Connection session save and restore
- Active connection name displayed in status bar

### v0.3.0 - Query Execution Core

- SQL editor with line numbers and gutter sync
- Cursor-position SQL statement extraction (extract_statement_at)
- Query execution for SQLite, PostgreSQL, and MySQL
- execute_with_cancel backed by CancellationToken
- Basic result table with loading indicator and 0-row placeholder
- Inline SQL error display in result area
- Execution time and row count in status bar
- HistoryService with SQLite persistence

### v0.4.0 - Schema Browser

- fetch_metadata for SQLite, PostgreSQL, and MySQL
- MetadataCache with memory and SQLite flush
- Background metadata fetch on connect
- Collapsible three-level schema tree in sidebar
- Table double-click inserts SELECT * FROM into editor
- Alt+Arrow pane focus navigation with visual focus borders
- Sidebar keyboard navigation (Up/Down/Left/Right/Enter)

### v0.5.0 - Result Table Polish

- Virtual scrolling via Slint ListView (handles large result sets)
- NULL cells rendered as a muted badge, distinct from empty string
- Client-side column sort (click header to toggle asc/desc)
- Bottom preview pane for full cell content
- Copy cell / row / TSV with headers (Ctrl+C + right-click menu)
- Pagination row-count selector (100 / 500 / 1000 / ALL)
- Result table keyboard navigation: row mode, cell mode, search/filter mode

### v0.6.0 - SQL Experience

- Completion engine: keyword, table, column, and alias-aware candidates
- CompletionService with 300ms debounce
- Completion popup UI in editor (Up/Down/Enter/Tab/Esc)
- SQL formatter (Ctrl+Shift+F) via wf-query

### v0.7.0 - Finishing Touches

- CSV export with save dialog and BOM header
- JSON export (null -> JSON null, numeric coercion)
- Dark/light theme switching (status bar toggle + ThemeColors global)
- Font family and font size configuration applied to editor and result table
- Bundled fonts: Inter, Noto Sans JP, JetBrains Mono
- Session restore: editor query text persisted and reloaded on startup
- Custom menu bar (File / Edit / Query / Settings dropdowns)
- Ctrl+Enter: execute statement at cursor
- Ctrl+Shift+Enter: execute entire editor content
- Structured logging with RUST_LOG control (tracing)
- Localization support: English and Japanese (Slint gettext + rust-i18n)
- Runtime language switching via config.toml [ui] language field

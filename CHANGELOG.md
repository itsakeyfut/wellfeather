# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.10.0] - 2026-05-25

### v0.10.0 - Advanced Connectivity, History & Undo

#### Added

- SSH tunnel support for database connections: configure host, port, user, and key file via the connection form (#313)
- SSL/TLS mutual authentication for database connections: CA certificate, client certificate, and client key (#314)
- Connection group folder organization in the sidebar: drag connections into named groups, persist across restarts (#315)
- Query history panel (Ctrl+Shift+H): search by keyword or connection, insert into editor, re-run directly (#317)
- `HistoryService::search` with keyword and connection filter backed by SQLite full-text index (#316)
- Panel-scoped undo/redo: text undo in SQL editor tabs, action undo for sidebar group operations (#319)
- Configurable query timeout with auto-cancel: Off / 10s / 30s / 60s, selectable in Editor Preferences (#320)
- Slow query detection: queries exceeding the configured threshold show a yellow warning in the status bar (#327)
- Unified Editor Preferences dialog: Font Family, Font Size, Theme, Reduce Motion, Tab Width, Slow Query threshold, Query Timeout, and Language in a single panel (#329)
- Full keyboard navigation for Editor Preferences dialog: Tab/Shift+Tab between groups, Left/Right within segment buttons, Enter/Space to confirm, Esc to cancel (#330)

#### Fixed

- Restrict `wellfeather.db` file permissions to owner-only (0600) on Unix at startup (#312)
- Sanitize CSV export cells to prevent formula injection (prefix `=`, `+`, `-`, `@` with a tab) (#311)
- Use `Zeroizing<Vec<u8>>` to guarantee decrypted passwords are zeroed on all exit paths (#310)
- Cap regex pattern length and automaton state count to prevent ReDoS in find/replace (#309)
- Validate SQLite file path to reject directory traversal sequences and special URI schemes (#308)
- Prevent phantom undo entries on the first keystroke in a new editor tab (#319)

#### Performance

- Move `QueryResult.rows` into `Arc` to eliminate double clone when rendering large result sets (#326)
- Add SQLite `PRAGMA` tuning (`journal_mode=WAL`, `synchronous=NORMAL`, `cache_size`) and aggregate `next_query_number` query (#325)
- Add indexes on `timestamp` and `last_used_at` columns in history tables to eliminate full table scans (#324)
- Await snippet list refresh instead of spawning unbounded tasks on every editor keystroke (#323)
- Offload SQL formatter to `spawn_blocking` to prevent blocking the Slint event thread (#322)
- Eliminate O(rows × cols) `String` allocations in `filter_rows` by using byte-level search (#321)

## [0.9.1] - 2026-05-08

### Fixed

- Suppress Windows console window appearing behind the GUI app in release builds

## [0.9.0] - 2026-05-07

### v0.9.0 - Editor Polish & Security Hardening

#### Added

- SQL syntax highlighting with a color overlay in the editor (keywords, identifiers, strings, comments, operators)
- Animation system with `reduce-motion` config option; all transitions respect the setting
- Floating find/replace bar with search history (Ctrl+F)
- Snippet system: save, browse, and insert reusable SQL snippets via a draggable panel
- INSERT SQL export from query results (complements existing CSV and JSON export)
- Metadata search palette (Ctrl+P): fuzzy-search tables, columns, and views
- Command palette (Ctrl+K): fuzzy-search connections and built-in actions
- Platform abstraction layer for OS directories, DPI, and native dark-mode detection
- Distribution packaging: Windows MSIX, macOS DMG, and Linux AppImage via `cargo x package`
- Tab key inserts spaces with configurable width (2 / 4 / 8) via Edit → Editor Preferences

#### Fixed

- Text-jump on Enter in the syntax-highlight overlay (stale `line-h` for one frame) (#252)
- Password redacted from controller command log and `ConnectionFailed` error messages (#268)
- Username and password in PostgreSQL/MySQL connection URLs are now percent-encoded (#269)
- Key file and directory permissions restricted to owner-only on Unix (0o600 / 0o700) (#270)
- Leading SQL comments (`--`, `/* */`) no longer bypass DML safety checks (#271, #296)
- Completion popup stays open after Backspace clears the word prefix (#260)
- Accepted completion item text was not syntax-highlighted until next keystroke (#301)
- Re-connecting to an already-active connection from the command palette is now a no-op

#### Changed

- All SQLite databases consolidated into a single `wellfeather.db` file

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

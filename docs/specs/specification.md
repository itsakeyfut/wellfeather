# Wellfeather Specification

> Last updated: 2026-04-30
>
> **Core Values**: Lightweight · Fast · Keyboard-centric · Query experience
> **Differentiation**: Faster startup, lower memory footprint, and superior keyboard operation compared to DBeaver / TablePlus / DataGrip

---

## 1. Project Overview

| Item | Detail |
|------|--------|
| App name | `wellfeather` |
| Target platforms | Windows / macOS / Linux (cross-platform) |
| Goal: startup time | < 1 second |
| Goal: memory | Minimized |
| UI responsiveness | Non-blocking (fully async) |

### Competitive Comparison

| | DBeaver | TablePlus | DataGrip | **wellfeather** |
|--|---------|-----------|----------|-----------------|
| Startup time | 10–30s | 2–3s | 10–20s | **< 1s** |
| Memory | 500MB+ | 150MB+ | 400MB+ | **Minimized** |
| Keyboard operation | △ | △ | ○ | **◎** |
| Vim keybindings | △(plugin) | × | △ | Phase 2 |
| Command palette | × | × | ○ | Phase 2 |
| SQL completion | ○ | △ | ◎ | Lightweight (MVP) |
| SQL formatter | △ | △ | ◎ | **◎(MVP)** |
| Virtual scroll | △ | ○ | ○ | **◎(MVP)** |
| NULL visibility | △ | ○ | ○ | **◎(MVP)** |
| Table double-click → INSERT SELECT | × | ○ | × | **○(MVP)** |

---

## 2. Technology Stack

| Item | Technology |
|------|-----------|
| Language | Rust |
| UI | Slint |
| DB connectivity | sqlx |
| Async runtime | tokio |
| Supported DBs | PostgreSQL / MySQL / SQLite |

---

## 3. Architecture

```
UI (Slint)
  ↓
Controller
  ↓
App State
  ↓
Query Engine (async)
  ↓
DB Driver (sqlx)
```

### Policy
- UI is responsible only for displaying state
- Logic is centralized on the Rust side
- Async processing is separated from the UI
- UI updates go through `slint::invoke_from_event_loop`

---

## 4. Directory Layout (Preliminary)

```
src/
├── main.rs
├── ui/
│   ├── app.slint
│   └── components/
│       ├── sidebar.slint
│       ├── editor.slint
│       ├── result_table.slint
│       └── status_bar.slint
├── app/
│   ├── state.rs
│   ├── controller.rs
│   └── session.rs          // session restore
├── db/
│   ├── connection.rs
│   ├── query_executor.rs
│   └── models.rs
├── query/
│   ├── history.rs          // SQLite persistence
│   ├── formatter.rs        // SQL formatter
│   └── analyzer.rs         // cursor position analysis (for completion)
└── completion/
    ├── metadata_provider.rs
    └── engine.rs
```

---

## 5. UI Layout

```
+---------------------------------------------------------------+
| Menu bar                                                      |
+-------------------+-------------------------------------------+
| Sidebar           | Query Editor                              |
|                   |  Line numbers | SQL input area            |
| ▼ my_postgres     |                                           |
|   ▼ Tables        +-------------------------------------------+
|     ▶ users       | Result Table (virtual scroll)             |
|     ▶ orders      |  NULL: badge display                      |
|   ▼ Views         |  Copy: Ctrl+C / right-click menu          |
|     ▶ active_...  +-------------------------------------------+
|   ▶ Stored Procs  | Bottom preview pane (expand long text)    |
|   ▶ Indexes       |                                           |
+-------------------+-------------------------------------------+
| Status Bar: [connection/DB] [exec time] [rows] [✓/✗ message] |
+---------------------------------------------------------------+
```

---

## 6. Connection Management

### Persistence

| Item | Detail |
|------|--------|
| Config file format | TOML |
| Storage location | OS standard config directory |
| &nbsp;&nbsp;Windows | `%APPDATA%\wellfeather\` |
| &nbsp;&nbsp;macOS | `~/Library/Application Support/wellfeather/` |
| &nbsp;&nbsp;Linux | `~/.config/wellfeather/` |
| File structure | `config.toml` (connection settings / app settings) |
| History DB | `history.db` (SQLite, same directory) |

### Connection Input Methods
Both supported (user's choice):
- Connection string: `postgres://user:pass@host:5432/dbname`
- Individual fields: host / port / user / password / database name

### Password Storage
- **Method**: Encrypted with AES-256-GCM and stored in `config.toml`
- App-specific key management to be detailed in the implementation phase

### Data Structures

```rust
pub enum DbType {
    PostgreSQL,
    MySQL,
    SQLite,
}

pub struct DbConnection {
    pub id: String,                          // UUID
    pub name: String,                        // display name
    pub db_type: DbType,
    // connection string mode
    pub connection_string: Option<String>,
    // individual field mode
    pub host: Option<String>,
    pub port: Option<u16>,
    pub user: Option<String>,
    pub password_encrypted: Option<String>,  // AES-256-GCM encrypted
    pub database: Option<String>,
}
```

### Multiple Connections
- Maintain simultaneous connections to multiple DBs
- List all connections in the sidebar; switch the active connection

---

## 7. Session Restore

Automatically restore the previous state on startup:
- Auto-connect to the last active connection
- Restore the last query string into the editor
- Connection info stored in the `[session]` section of `config.toml`

---

## 8. Sidebar

### Display Items (Tree Structure)

```
▼ [Connection: my_postgres]
  ▼ Tables
    ▶ users
    ▶ orders
  ▼ Views
    ▶ active_users
  ▼ Stored Procedures
    ▶ get_user_by_id
  ▼ Indexes
    ▶ users_pkey
▼ [Connection: local_sqlite]
  ...
```

- Each node is collapsible / expandable
- All item types shown (Tables / Views / Stored Procedures / Indexes)

### Interaction
- **Double-click** a table name → insert `SELECT * FROM <table_name>` into the editor

---

## 9. SQL Editor

| Item | Detail |
|------|--------|
| Line numbers | Displayed (MVP) |
| Syntax highlighting | Phase 2 |
| Multi-tab | Phase 2 |
| Vim keybindings | Phase 2 |
| Completion trigger | Auto popup + `Ctrl+Space` both supported |
| SQL formatter | Format with `Ctrl+Shift+F` (MVP) |
| Font settings | Configurable via `config.toml` (MVP) |

### Keyboard Shortcuts (Confirmed)

| Key | Action |
|-----|--------|
| `Ctrl+Enter` | Execute only the SQL statement at the cursor |
| `Ctrl+Shift+Enter` | Execute the entire editor content |
| `Shift+Enter` | Execute selected range only |
| `Ctrl+Space` | Manually show completion candidates |
| `Ctrl+Shift+F` | Format SQL (formatter) |
| `Esc` | Cancel running query |
| `Alt+↑/↓/←/→` | Move between panes |

---

## 10. SQL Completion

### Policy
- Prioritize lightweight and fast (heavy LSP integration is Phase 2+)
- Cache DB metadata in local memory on connect

### Components
- **Metadata Provider**: Fetch table / column / view / index info on connect
- **Completion Engine**:
  - SQL keyword completion (`SELECT`, `WHERE`, etc.)
  - Table name completion
  - Column completion (filtered by `FROM` clause)
- **Parser (lightweight)**: Analyze context at cursor position

### Triggers
- Auto popup while typing
- Also manually invocable with `Ctrl+Space`

---

## 11. Query Execution

### Execution Flow

```
1. UI calls run_query (Ctrl+Enter / Ctrl+Shift+Enter / Shift+Enter)
2. Controller receives it and starts execution
3. Async execution in a tokio task
4. Reflect result in AppState
5. Update UI via slint::invoke_from_event_loop
6. Display execution time and row count in the status bar
```

### Query Cancellation
- **MVP**: Cancel with `Esc` key or "Cancel" button during execution
- **Phase 2**: Timeout setting (e.g., auto-cancel after 3 minutes)

### Data Structures

```rust
pub async fn execute_query(sql: &str, conn: &DbConnection) -> Result<QueryResult>

pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,  // NULL is None
    pub row_count: usize,
    pub execution_time_ms: u128,
}

pub struct QueryExecution {
    pub id: i64,
    pub sql: String,
    pub duration_ms: u128,
    pub success: bool,
    pub error_message: Option<String>,
    pub timestamp: i64,
    pub connection_id: String,
}
```

---

## 12. Result Table

| Item | Detail |
|------|--------|
| Rendering | Virtual scroll (render only visible range; handles large row counts) |
| Pagination | User-selectable (100 / 500 / 1000 rows) |
| Column sort | Client-side sort (MVP) |
| NULL display | Badge/pill UI (small `NULL` label, high visibility) |
| Data editing | Read-only (MVP) |
| Export | CSV + JSON (MVP) |
| Copy | `Ctrl+C`: cell value; right-click menu: cell value / entire row / TSV format |
| Bottom preview pane | Always shows full content of the selected cell (supports long text and JSON) |

---

## 13. Error Display

- **Method**: Inline display in the result area (no dialogs)
- Error summary also shown in the status bar

---

## 14. Status Bar

Displayed left to right:

```
[ Connection: my_postgres / dbname ] [ 42 ms ] [ 128 rows ] [ ✓ Success / ✗ Error message ]
```

---

## 15. Query History

| Item | Detail |
|------|--------|
| Storage | SQLite (`history.db`, OS config directory) |
| Max entries | Unlimited |
| Search / filter | Phase 2 |
| Display UI | Phase 2 |

---

## 16. App Settings (`config.toml`)

```toml
[appearance]
theme = "dark"          # "dark" | "light"
font_family = "JetBrains Mono"
font_size = 14

[editor]
page_size = 500         # 100 | 500 | 1000

[session]
last_connection_id = "uuid-xxx"
last_query = "SELECT * FROM users"

[[connections]]
id = "uuid-xxx"
name = "my_postgres"
db_type = "postgresql"
host = "localhost"
port = 5432
user = "admin"
password_encrypted = "AES256GCM:..."
database = "mydb"
```

---

## 17. App State

```rust
pub struct AppState {
    pub connections: Vec<DbConnection>,
    pub active_connection_id: Option<String>,
    pub current_query: String,
    pub result: Option<QueryResult>,
    pub is_loading: bool,
    pub cancel_token: Option<CancellationToken>, // for query cancellation
    pub history: Vec<QueryExecution>,
    pub theme: Theme,
    pub page_size: usize,
}

pub enum Theme {
    Dark,
    Light,
}
```

---

## 18. MVP Feature Scope

### Included in MVP (Phase 1)

| Feature | Notes |
|---------|-------|
| DB connection (Postgres / MySQL / SQLite) | |
| Persist connection info (TOML + AES-256-GCM encryption) | |
| Multiple simultaneous connections + switching | |
| Session restore (last connection + query) | |
| Sidebar (tree structure) | Tables / Views / Stored Procs / Indexes |
| Table double-click → `SELECT * FROM` insert | |
| SQL editor (with line numbers) | |
| Query execution (cursor statement / full / selection) | |
| Query cancellation (Esc / Cancel button) | |
| SQL completion (lightweight metadata-based) | |
| SQL formatter (Ctrl+Shift+F) | |
| Result table (virtual scroll) | |
| NULL badge display | |
| Column sort (client-side) | |
| Bottom preview pane (expand long text) | |
| Copy (cell value / row / TSV) | |
| Export (CSV + JSON) | |
| Status bar | |
| Inline error display | |
| Query history persistence (SQLite) | |
| Theme switching (dark/light) | |
| Font settings (config.toml) | |

### Phase 2

| Feature |
|---------|
| Syntax highlighting |
| Multi-tab |
| Vim keybindings |
| Command palette |
| History search / filter UI |
| Slow query detection |
| EXPLAIN display |
| Query timeout setting (auto-cancel) |
| History search UI |

### Phase 3

| Feature |
|---------|
| Lightweight analysis / performance hints |
| Index advisor |
| OS keychain integration |
| CLI integration |
| LSP-based SQL completion |

---

## 19. Intentionally Out of Scope for MVP

- Advanced metrics
- Complex plugins
- Heavy UI
- Cell editing / UPDATE issuance
- Dialog-style error display

---

## 20. Non-Functional Requirements

| Item | Requirement |
|------|-------------|
| Startup time | < 1 second |
| Memory usage | Minimized |
| UI responsiveness | Non-blocking |
| Large data | Handle tens of thousands of rows via virtual scroll |

---

## 21. Localization (i18n)

### Supported Languages

| Code | Language |
|------|----------|
| `en` | English (default) |
| `ja` | Japanese |

The language is specified via the `[ui] language` field in `config.toml`. It is loaded at startup and applied globally.

### Hybrid i18n Architecture

Different tools are used for UI strings and Rust-side messages:

| Target | Tool | File format | Macro |
|--------|------|-------------|-------|
| UI strings inside `.slint` | Slint built-in i18n (GNU gettext) | `.po` | `@tr("key")` |
| Rust-side messages (errors, lifecycle) | `rust-i18n` crate | `.yml` | `t!("ns.key")` |

### File Layout

```
app/
├── lang/
│   ├── en/LC_MESSAGES/wellfeather.po   ← Slint UI strings (English)
│   └── ja/LC_MESSAGES/wellfeather.po   ← Slint UI strings (Japanese)
└── locales/
    ├── en.yml                           ← Rust-side messages (English)
    └── ja.yml                           ← Rust-side messages (Japanese)
```

### Initialization

```rust
// app/src/main.rs or lib.rs
slint::init_translations!(concat!(env!("CARGO_MANIFEST_DIR"), "/lang"));
slint::select_bundled_translation(&config.ui.language);

rust_i18n::i18n!("locales", fallback = "en");
rust_i18n::set_locale(&config.ui.language);
```

### `.yml` Naming Convention

```yaml
en:
  app:
    ready: "Ready"
  error:
    db_connect_failed: "Failed to connect: %{reason}"
    query_timeout: "Query timed out after %{seconds}s"
  query:
    cancelled: "Query cancelled"
  config:
    load_failed: "Failed to load config: %{reason}"
```

### `LocalizedMessage` Trait

All errors displayed in the UI must implement the `LocalizedMessage` trait, returning a translated string via `t!()`.

```rust
pub trait LocalizedMessage {
    fn localized_message(&self) -> String;
}
```

The error types in `wf-db`, `wf-config`, `wf-query`, `wf-completion`, and `wf-history` each implement this trait.

### `.slint` String Rules

All user-facing strings in `.slint` files must use `@tr("key")`. Hardcoded English strings are prohibited.

---

---

## 22. Connection Security (v0.9.0)

### 22-1. SSH Tunnel

Added via a **"SSH Tunnel"** tab in the connection dialog.

| Field | Detail |
|-------|--------|
| SSH host / port | Defaults: port 22 |
| SSH user | Required |
| Auth method | Private key (file + passphrase) or Password |
| Password storage | AES-256-GCM |
| Library | `ssh2` crate (vendored OpenSSL for Windows) |

**Behavior**: On connect, an unused local port is allocated by the OS → SSH tunnel established → DB connection routed through that port. Tunnel closed on disconnect.

**Known Hosts**: First connect shows the SHA-256 fingerprint for user confirmation. Approved hosts written to `{config_dir}/known_hosts`. Mismatch blocks connection.

### 22-2. SSL/TLS

Added via a **"SSL/TLS"** tab in the connection dialog.

| Field | Detail |
|-------|--------|
| CA certificate | Optional, PEM |
| Client certificate | Optional, PEM |
| Client private key | Optional, PEM |
| PostgreSQL sslmode | `require` / `verify-ca` / `verify-full` |
| MySQL TLS mode | `REQUIRED` / `VERIFY_CA` / `VERIFY_IDENTITY` |

Certificate files are copied to `{config_dir}/certs/{connection_id}/` on save so the original path can move freely.

### 22-3. SOCKS5 Proxy (v0.10.0)

Integrated into the SSH Tunnel tab as a **tunnel-type radio**: None / SSH Tunnel / SOCKS5.

SOCKS5 fields: proxy host, port (default 1080), optional username + password (AES-256-GCM).

### 22-4. Cloud IAM Authentication (v0.12.0)

Added via a **"Cloud IAM"** tab in the connection dialog. Provider radio: None / AWS / GCP / Azure.

| Provider | Method |
|----------|--------|
| AWS RDS | `aws-sdk-rust` standard credential chain; RDS IAM token (15 min TTL), auto-renewed |
| GCP Cloud SQL | `cloud-sql-proxy` subprocess; local port tunnel |
| Azure SQL | `az account get-access-token` subprocess; token auto-renewed (~1 h TTL) |

### 22-5. Connection Groups (v0.9.0)

One level of folder nesting: Group → Connection. Config:

```toml
[[group]]
id = "uuid"
name = "Production"
color = "#e53935"

[[connection]]
group_id = "uuid"
```

Color inherits from group; per-connection color overrides.

---

## 23. UI/UX Enhancements (v0.8.0)

### 23-1. DDL Viewer

Single-clicking a sidebar object (table / view / index) replaces the bottom pane with the object's `CREATE` statement. Clicking a cell restores cell preview. Read-only code view with copy button and syntax highlighting.

DB implementation:
- SQLite: `SELECT sql FROM sqlite_master WHERE name = ?`
- PostgreSQL: `information_schema` + `pg_catalog` reconstruction
- MySQL: `SHOW CREATE TABLE` / `SHOW CREATE VIEW`

### 23-2. Safe DML Mode

`ConnectionConfig.safe_dml: bool` (default `true`). If a `UPDATE`/`DELETE` statement lacks a `WHERE` clause, a confirmation dialog is shown before execution. Detected by `wf-query` just before the DB call. `TRUNCATE`/`DROP` are handled by read-only mode.

### 23-3. Read-Only Mode

`ConnectionConfig.read_only: bool`. Blocks INSERT / UPDATE / DELETE / DDL before execution. Lock icon (🔒) on connection name in sidebar and status bar indicator. Error shown in status bar, no dialog.

### 23-4. Find / Replace Bar

`Ctrl+F`: find bar. `Ctrl+H`: find + replace bar. Floats over editor text (does not push content down). Features: case-sensitive toggle, regex toggle, match count (`3 / 12`), next/prev navigation, replace-one and replace-all. `Esc` or ✕ to close.

### 23-5. Query Bookmarks

Saved to `bookmarks.toml` (same directory as `config.toml`):

```toml
[[bookmark]]
id = "uuid"
name = "Monthly Summary"
folder = "Reports"
connection_id = "uuid"   # omit for global
sql = "SELECT ..."
created_at = "2026-04-30T..."
```

Sidebar "Bookmarks" section (above connection tree). Double-click to load into editor. `Ctrl+D` or right-click "Save as bookmark" to save current SQL. Right-click: rename / delete / move to folder.

### 23-6. Connection Color Coding

`ConnectionConfig.color: Option<String>` (CSS color code). 12-color fixed palette in the connection edit dialog. Color dot (●) shown left of connection name in sidebar and in the status bar active-connection display.

### 23-7. Metadata Global Search

`Ctrl+P` opens a floating search palette. Searches all tables / views / columns in `MetadataCache` with incremental substring matching (prefix-first ranking).

- Table/view selected → sidebar focus + DDL shown in bottom pane
- Column selected → parent table focused + column name copied to clipboard

---

## 24. Advanced Editor Features (v0.10.0)

### 24-1. Multi-Tab Editor

`TabEntry { id, name, query_text, result }` replaces the single-editor model. `AppState.tabs: Vec<TabEntry>`.

| Shortcut | Action |
|----------|--------|
| `Ctrl+T` | New tab |
| `Ctrl+W` | Close tab (no confirm) |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` | Next / previous tab |
| `Ctrl+1`–`9` | Jump to tab N |
| Double-click tab name | Rename inline |

All tabs share the active connection. Tab SQL text, name, and order persisted in `session.toml`.

### 24-2. Code Snippets

Saved to `snippets.toml`. Sidebar "Snippets" section mirrors the Bookmarks pattern. `Ctrl+Shift+S` opens a fuzzy-search palette for quick insert at cursor position.

### 24-3. Parameterized Queries

Syntax: `:name` (e.g. `SELECT * FROM users WHERE id = :id`). `wf-query` extracts parameter names; a dialog shows all variables (name, type dropdown, value input) before execution. Previous values restored from `QueryExecution` history. `:name` tokens highlighted in a distinct editor color.

---

## 25. Data Editing (v0.10.0)

### 25-1. Inline Cell Editing

Available only when the result set contains a PK column. Double-click enters edit mode (highlighted border). Uncommitted changes shown with a colored background. Toolbar shows "Commit ✓" / "Rollback ✗" buttons only when uncommitted changes exist.

- NULL assignment: clear cell content then press `Delete` → `[NULL]` badge
- Row add: "＋ Add row" toolbar button → empty row appended
- Row delete: `Delete` key or right-click "Delete row" → strikethrough → committed as DELETE

### 25-2. Form View

Toggle button (grid icon / form icon) in result table toolbar. Shows one record at a time in `column: value` vertical layout. Navigation: ← → toolbar buttons or `Alt+↑/↓` keyboard. Uses the same transaction control as inline cell editing.

### 25-3. JSON/XML Tree Viewer

On cell selection, the value is attempted as JSON then XML parse. On success the bottom pane switches to a collapsible tree view. Type-colored nodes for JSON (string: green, number: blue, boolean: orange, null: gray). Right-click node: "Copy path" or "Copy value". "View as text" button reverts to plain text.

---

## 26. Schema Analysis Tools (v0.11.0)

### 26-1. Visual Filter Builder

Client-side filter panel that slides in above the result table. Each filter row: `[column▼] [operator▼] [value] [+] [✕]`. Operators: `=`, `≠`, `>`, `<`, `≥`, `≤`, `LIKE`, `NOT LIKE`, `IS NULL`, `IS NOT NULL`. AND/OR radio for all conditions. Applies in real time. "Copy as WHERE clause" button.

### 26-2. EXPLAIN Visualizer

Toolbar "Explain" button (left of Run). Results appear in a new "EXPLAIN" tab next to "Results". DB-specific execution:

| DB | Command |
|----|---------|
| PostgreSQL | `EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)` |
| MySQL | `EXPLAIN ANALYZE` or `EXPLAIN FORMAT=JSON` |
| SQLite | `EXPLAIN QUERY PLAN` |

Unified `ExplainNode` tree. Seq Scan nodes highlighted red; index scan nodes highlighted green. Node click shows details (buffer usage, loop count) in a side panel.

### 26-3. Table Structure Editor + Index Management

Right-click table in sidebar → "Edit Structure". Two-tab dialog:

- **Columns**: grid of name/type/NOT NULL/default/comment. Inline edit, add, delete-mark.
- **Indexes**: list with name/columns/UNIQUE/type. Add form, delete-mark.

"Preview" button shows generated DDL. "Apply" executes and refreshes metadata.

### 26-4. ER Diagram

Rendered natively via **Slint Canvas API** (no WebView). Layout via **Fruchterman–Reingold** algorithm (Rust, max 100 iterations). Nodes: table cards with column list. Edges: FK arrows with cardinality labels.

Interactions: node drag, scroll-to-zoom, double-click to sidebar focus + DDL display. PNG/SVG export. Accessible via menu "Database → ER Diagram" or sidebar schema right-click.

### 26-5. Schema Diff

Menu "Database → Schema Diff". Select two live connections. Three-column display (A DDL | change icon | B DDL). Change types: Added (green +), Removed (red −), Modified (yellow ∼). Objects: tables, columns, indexes, views. "Generate Migration Script" inserts `ALTER TABLE` / `CREATE INDEX` DDL into the editor.

---

## 27. Data Operations (v0.8.0 / v0.12.0)

### 27-1. INSERT SQL Export (v0.8.0)

File → Export → Insert SQL. Scope: current page or all rows (with large-table warning). Output: named-column batch INSERT with NULL literals and escaped strings. Table name auto-detected for single-table queries.

### 27-2. CSV/JSON Import (v0.12.0)

File → Import. Guided dialog: file picker → CSV options (delimiter, encoding) → header detection → column mapping with type-mismatch warnings → 10-row preview → error mode (skip / rollback). Progress bar with cancel. JSON: root-array format only.

### 27-3. Full Schema Dump (v0.12.0)

File → Dump Database. Scope: all tables or selected tables. Content: DDL only / data only / both.

| DB | Tool |
|----|------|
| PostgreSQL | `pg_dump` (PATH auto-detect + manual override; `PGPASSWORD` env) |
| MySQL | `mysqldump` (same) |
| SQLite | App-internal (`sqlite_master` DDL + INSERT generation) |

---

## 28. Cloud Data Warehouse Support (v1.0.0)

### 28-1. BigQuery

Authentication: Service Account JSON file path (stored AES-256-GCM encrypted). Schema browser: project → dataset → table (3-level). Result pagination via `pageToken`. Optional dry-run byte estimate before execution.

### 28-2. Snowflake

Authentication: username/password (JWT optional). Connection fields: account identifier, warehouse, database, schema, role. Schema browser: database → schema → table. VARIANT columns auto-linked to JSON tree viewer.

### 28-3. Amazon Redshift

PostgreSQL-compatible driver (`sqlx` PG + SSL required). Redshift-specific catalog queries (`pg_table_def`, `SVV_COLUMNS`). Added as a distinct connection type in the UI.

---

## 29. AI SQL Generation (v1.0.0)

A collapsible side panel on the right edge of the editor. Toggle button "✦ AI" in the editor toolbar. Open/closed state persists in `session.toml`.

### Panel Layout

```
┌──────────────────────────────────────┐
│ ✦ AI SQL Assistant     [× collapse] │
├──────────────────────────────────────┤
│ Prompt:                               │
│ [ natural language input...         ] │
│ [Generate]                            │
├──────────────────────────────────────┤
│ Preview (streaming):                  │
│ [ generated SQL shown here...       ] │
│ [Insert into Editor]  [Copy]          │
├──────────────────────────────────────┤
│ History:                              │
│ > Monthly summary query               │
│ > User list with filters              │
└──────────────────────────────────────┘
```

### Behavior

- **Prompt**: `Ctrl+Enter` to submit
- **Schema context**: Full `MetadataCache` contents sent automatically. If table count exceeds the token budget, truncated to top-N tables (alphabetical), with a "Schema partially omitted" notice
- **Model**: `claude-sonnet-4-6` (fixed; future model selection possible)
- **Streaming**: tokens displayed progressively via `slint::invoke_from_event_loop`
- **Insert**: inserts into the cursor position of the active tab
- **API key**: stored in `config.toml` `[ai].api_key` (AES-256-GCM encrypted); guidance shown when unset

---

## 30. Planned Future Features (v1.x)

### 30-1. NoSQL Support (MongoDB / Redis)

- **MongoDB**: collection browser, JSON document viewer, find/aggregate execution
- **Redis**: key browser, type-specific viewers (String/List/Hash/Set/ZSet), command execution
- `wf-db` `DbPool` enum extended with NoSQL variants, or a separate NoSQL driver layer added

---

## Change Log

| Date | Description |
|------|-------------|
| 2026-03-31 | Initial version: core specification |
| 2026-03-31 | Detailed specification based on competitive analysis (virtual scroll, NULL display, formatter, etc.) |
| 2026-04-01 | Added §21 Localization spec (Slint i18n + rust-i18n hybrid, en/ja support) |
| 2026-04-30 | Added §22–30: connection security, UI enhancements, editor features, data editing, schema tools, data operations, cloud DW, AI SQL generation, future NoSQL — covering milestones v0.8.0 through v1.x |

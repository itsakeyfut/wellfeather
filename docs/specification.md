# Wellfeather Specification

> Last updated: 2026-03-31
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

## Change Log

| Date | Description |
|------|-------------|
| 2026-03-31 | Initial version: core specification |
| 2026-03-31 | Detailed specification based on competitive analysis (virtual scroll, NULL display, formatter, etc.) |
| 2026-04-01 | Added §21 Localization spec (Slint i18n + rust-i18n hybrid, en/ja support) |

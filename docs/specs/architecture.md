# Wellfeather Architecture Design

> Last updated: 2026-04-01

---

## 1. Overall Architecture

```
┌─────────────────────────────────────────────────────┐
│                   UI Layer (Slint)                  │
│  sidebar.slint / editor.slint / result_table.slint  │
└──────────────────┬──────────────────────────────────┘
                   │  Slint callbacks + invoke_from_event_loop
┌──────────────────▼──────────────────────────────────┐
│               AppController                         │
│  Receive Command → Call Service → Send Event        │
└──────┬───────────┬────────────────┬─────────────────┘
       │           │                │
┌──────▼──┐  ┌─────▼──────┐  ┌─────▼──────────┐
│DbService│  │HistoryService│  │CompletionService│
└──────┬──┘  └─────┬──────┘  └─────┬──────────┘
       │           │                │
┌──────▼──┐  ┌─────▼──────┐  ┌─────▼──────────┐
│ DbPool  │  │ history.db │  │MetadataCache   │
│(enum)   │  │ (SQLite)   │  │(Memory+SQLite) │
└─────────┘  └────────────┘  └────────────────┘
```

---

## 2. Communication Pattern: Controller + Channel Hybrid

The UI and backend communicate over two directional channels: **Command** and **Event**.
The UI only sends Commands; the Controller only returns Events. UI updates always go through `invoke_from_event_loop`.

```
UI (Slint)
  │  tx_cmd.send(Command::RunQuery(sql))
  │
  ▼
AppController (looping in a tokio task)
  │  rx_cmd.recv() → match cmd → call Service
  │  tx_event.send(Event::QueryResult(...))
  │
  ▼
UI update handler (invoke_from_event_loop)
  │  rx_event.recv() → match event → set Slint property
```

### Command / Event Definitions

```rust
/// UI → Controller
pub enum Command {
    Connect(DbConnection),
    Disconnect(String),           // connection_id
    RunQuery(String),             // sql
    RunSelection(String),         // sql (selected range)
    RunAll(String),               // sql (entire editor)
    CancelQuery,
    FetchCompletion(String, usize), // sql, cursor_pos
    ExportResult(ExportFormat, PathBuf),
    UpdateConfig(ConfigUpdate),
}

/// Controller → UI
pub enum Event {
    Connected(String),            // connection_id
    Disconnected(String),
    QueryStarted,
    QueryFinished(QueryResult),
    QueryCancelled,
    QueryError(String),
    CompletionReady(Vec<CompletionItem>),
    MetadataLoaded(DbMetadata),
    ConfigUpdated,
    StateChanged(StateEvent),
}

pub enum StateEvent {
    QueryStarted,
    QueryFinished(QueryResult),
    ConnectionChanged(String),
    ThemeChanged(Theme),
    LoadingChanged(bool),
}

pub enum ExportFormat {
    Csv,
    Json,
}
```

---

## 3. State Management

### Design Policy
- `Arc<AppState>` is shared across all services (no outer lock)
- Each sub-state holds an internal `RwLock<Data>`
- Controllers and the UI never touch `RwLock` directly — method-only access
- State changes are sent as `StateEvent` through the channel to unify UI updates

### Structure

```rust
pub struct AppState {
    pub conn:  ConnectionState,
    pub query: QueryState,
    pub ui:    UiState,
}

pub type SharedState = Arc<AppState>;
```

### ConnectionState

```rust
pub struct ConnectionState {
    data: RwLock<ConnectionData>,
}

struct ConnectionData {
    connections:   Vec<DbConnection>,
    active_id:     Option<String>,
}

impl ConnectionState {
    pub fn active(&self) -> Option<DbConnection> { ... }
    pub fn add(&self, conn: DbConnection) { ... }
    pub fn set_active(&self, id: &str) { ... }
}
```

### QueryState

```rust
pub struct QueryState {
    data: RwLock<QueryData>,
}

struct QueryData {
    current_query:  String,
    result:         Option<QueryResult>,
    is_loading:     bool,
    cancel_token:   Option<CancellationToken>,
}

impl QueryState {
    pub fn set_loading(&self, v: bool) { ... }
    pub fn set_result(&self, r: QueryResult) { ... }
    pub fn set_cancel_token(&self, t: CancellationToken) { ... }
    pub fn cancel(&self) { ... }
}
```

### UiState

```rust
pub struct UiState {
    data: RwLock<UiData>,
}

struct UiData {
    theme:     Theme,
    page_size: usize,
    font:      FontConfig,
}

impl UiState {
    pub fn theme(&self) -> Theme { ... }
    pub fn set_theme(&self, t: Theme) { ... }
}
```

---

## 4. DB Layer Design

### DbPool (enum dispatch)

```rust
pub enum DbKind {
    Postgres,
    MySql,
    Sqlite,
}

pub enum DbPool {
    Pg(sqlx::PgPool),
    My(sqlx::MySqlPool),
    Sqlite(sqlx::SqlitePool),
}

impl DbPool {
    pub async fn execute(&self, sql: &str) -> Result<QueryResult, DbError> {
        match self {
            Self::Pg(p)     => pg::execute(p, sql).await,
            Self::My(p)     => my::execute(p, sql).await,
            Self::Sqlite(p) => sqlite::execute(p, sql).await,
        }
    }

    pub async fn fetch_metadata(&self) -> Result<DbMetadata, DbError> {
        match self {
            Self::Pg(p)     => pg::fetch_metadata(p).await,
            Self::My(p)     => my::fetch_metadata(p).await,
            Self::Sqlite(p) => sqlite::fetch_metadata(p).await,
        }
    }

    pub fn explain_prefix(&self) -> &'static str {
        match self {
            Self::Pg(_)     => "EXPLAIN ANALYZE",
            Self::My(_)     => "EXPLAIN",
            Self::Sqlite(_) => "EXPLAIN QUERY PLAN",
        }
    }
}
```

> **Evolution path**: When DB-specific differences grow, introduce a `trait QueryExecutor` internally to separate implementations.

### DbService

```rust
pub struct DbService {
    pools: HashMap<String, DbPool>,  // connection_id → pool
    state: SharedState,
}

impl DbService {
    pub async fn connect(&self, conn: &DbConnection) -> Result<(), DbError>;
    pub async fn execute(&self, conn_id: &str, sql: &str) -> Result<QueryResult, DbError>;
    pub async fn execute_with_cancel(
        &self, conn_id: &str, sql: &str, token: CancellationToken
    ) -> Result<QueryResult, DbError>;
    pub async fn fetch_metadata(&self, conn_id: &str) -> Result<DbMetadata, DbError>;
}
```

---

## 5. Error Handling

### Policy
- Each crate (`wf-db`, `wf-query`, etc.): typed error definitions via `thiserror`
- `AppController` and above: contextual error handling via `anyhow`

```rust
// wf-db/src/error.rs
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),
    #[error("Query execution error: {0}")]
    QueryError(String),
    #[error("Query cancelled")]
    Cancelled,
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
}

// app/src/app/controller.rs (using anyhow)
pub async fn run_query(&self, sql: &str) -> anyhow::Result<()> {
    let result = self.db.execute(conn_id, sql)
        .await
        .context("Query execution failed")?;
    // ...
}
```

---

## 6. SQL Completion Design

```
Input event
  │
  ├─ debounce (300ms)
  │
  ▼
CompletionService.complete(sql, cursor_pos)
  │
  ├─ Parser: analyze context at cursor position
  │    - determine keyword / table name / column name
  │
  ├─ Fetch candidates from MetadataCache (in-memory)
  │    - table list / column list (filtered by FROM clause)
  │
  └─ Result: Vec<CompletionItem>
```

### MetadataCache

```rust
pub struct MetadataCache {
    memory: RwLock<HashMap<String, DbMetadata>>,  // conn_id → metadata
    db_path: PathBuf,                             // SQLite flush destination
}

impl MetadataCache {
    pub async fn load(&self, conn_id: &str) -> Option<DbMetadata>;
    pub async fn store(&self, conn_id: &str, meta: DbMetadata);
    pub async fn flush_to_disk(&self) -> anyhow::Result<()>;  // called periodically
}
```

---

## 7. Query Cancellation Design

Uses `tokio_util::sync::CancellationToken`.

```rust
// At query execution
let token = CancellationToken::new();
state.query.set_cancel_token(token.clone());

tokio::select! {
    result = db.execute(sql) => { /* normal completion */ }
    _ = token.cancelled()   => { /* cancelled */ }
}

// On Esc / Cancel button
state.query.cancel();  // → internally calls token.cancel()
```

---

## 8. Configuration Management

```rust
pub struct ConfigManager {
    path: PathBuf,  // OS config directory / config.toml
}

impl ConfigManager {
    pub fn load() -> anyhow::Result<Config>;
    pub fn save(&self, config: &Config) -> anyhow::Result<()>;  // save immediately
    pub fn app_dir() -> PathBuf {
        // resolved via the `directories` crate
        // Windows: %APPDATA%\wellfeather
        // macOS:   ~/Library/Application Support/wellfeather
        // Linux:   ~/.config/wellfeather
    }
}
```

---

## 9. Session Restore

Startup sequence:

```
1. ConfigManager::load() → Config (connection list + last session info)
2. Auto-connect to the last active connection ID
3. Restore the last query string into the editor
4. Load MetadataCache from SQLite (background)
5. Display UI
```

---

## 10. Workspace Structure

Uses a Cargo workspace. `app/` is the only binary crate that depends on Slint;
crates under `crates/` do not depend on Slint.

### Crate Dependency Graph

```
wf-db ──────────────────────────────┐
wf-config ──────────────────────────┤
wf-query  ──────────────────────────┼──→ app  (+ slint)
wf-completion ──────────────────────┤
wf-history ─────────────────────────┘
```

### Crate Responsibilities

| Crate | Responsibility |
|-------|---------------|
| `wf-db` | DB connection, query execution, schema retrieval. `DbPool`, `DbService`, pg/my/sqlite drivers, `DbError`, `DbConnection`, `QueryResult`, `DbMetadata` |
| `wf-config` | Config file management + password encryption. `Config` struct, `ConfigManager`, AES-256-GCM crypto |
| `wf-query` | SQL utilities. Cursor position analysis (`analyzer`), SQL formatter, CSV/JSON export |
| `wf-completion` | Full SQL completion. `CompletionService`, `MetadataCache`, `CompletionEngine`, `parser` |
| `wf-history` | SQLite persistence of query history. `HistoryService`, `QueryExecution` |
| `app` | Slint UI + tokio startup + `AppController` + `AppState` + `Command/Event` + session restore |

### Directory Layout

```
wellfeather/
├── Cargo.toml                   # workspace root
│
├── app/                         # binary crate (only crate that depends on Slint)
│   ├── Cargo.toml
│   ├── build.rs                 # slint_build::compile
│   └── src/
│       ├── main.rs              # tokio startup + slint::run_event_loop
│       ├── app/
│       │   ├── controller.rs    # receive Command → Service → send Event
│       │   ├── command.rs       # Command enum
│       │   ├── event.rs         # Event / StateEvent enum
│       │   └── session.rs       # session restore logic
│       ├── state/
│       │   ├── mod.rs           # AppState / SharedState
│       │   ├── connection_state.rs
│       │   ├── query_state.rs
│       │   └── ui_state.rs
│       └── ui/
│           ├── mod.rs           # register_*_callbacks()
│           ├── app.slint
│           └── components/
│               ├── sidebar.slint
│               ├── editor.slint
│               ├── result_table.slint
│               └── status_bar.slint
│
└── crates/
    ├── wf-db/
    │   ├── Cargo.toml
    │   └── src/
    │       ├── lib.rs
    │       ├── pool.rs          # DbPool enum / DbKind enum
    │       ├── service.rs       # DbService
    │       ├── models.rs        # DbConnection, QueryResult, DbMetadata, etc.
    │       ├── error.rs         # DbError (thiserror)
    │       └── drivers/
    │           ├── pg.rs
    │           ├── my.rs
    │           └── sqlite.rs
    │
    ├── wf-config/
    │   ├── Cargo.toml
    │   └── src/
    │       ├── lib.rs
    │       ├── models.rs        # Config / AppearanceConfig / etc.
    │       ├── manager.rs       # ConfigManager
    │       └── crypto.rs        # AES-256-GCM password encryption
    │
    ├── wf-query/
    │   ├── Cargo.toml
    │   └── src/
    │       ├── lib.rs
    │       ├── analyzer.rs      # extract SQL statement from cursor position
    │       ├── formatter.rs     # SQL formatter
    │       └── export.rs        # CSV / JSON export
    │
    ├── wf-completion/
    │   ├── Cargo.toml
    │   └── src/
    │       ├── lib.rs
    │       ├── service.rs       # CompletionService (debounce)
    │       ├── engine.rs        # CompletionEngine
    │       ├── cache.rs         # MetadataCache (Memory + SQLite flush)
    │       └── parser.rs        # cursor position context analysis
    │
    └── wf-history/
        ├── Cargo.toml
        └── src/
            ├── lib.rs
            └── service.rs       # HistoryService (SQLite)
```

---

## 11. Key Dependencies

### workspace root (`Cargo.toml`)

```toml
[workspace]
members = ["app", "crates/wf-db", "crates/wf-config", "crates/wf-query", "crates/wf-completion", "crates/wf-history"]
resolver = "2"

[workspace.dependencies]
tokio       = { version = "1", features = ["full"] }
tokio-util  = "0.7"
serde       = { version = "1", features = ["derive"] }
anyhow      = "1"
thiserror   = "1"
tracing     = "0.1"
sqlx        = { version = "0.8", features = ["postgres", "mysql", "sqlite", "runtime-tokio", "macros"] }
uuid        = { version = "1", features = ["v4"] }
chrono      = { version = "0.4", features = ["serde"] }
```

### `app/Cargo.toml`

```toml
[dependencies]
slint       = "1"
wf-db       = { path = "../crates/wf-db" }
wf-config   = { path = "../crates/wf-config" }
wf-query    = { path = "../crates/wf-query" }
wf-completion = { path = "../crates/wf-completion" }
wf-history  = { path = "../crates/wf-history" }
tokio       = { workspace = true }
tokio-util  = { workspace = true }
anyhow      = { workspace = true }
tracing     = { workspace = true }
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

[build-dependencies]
slint-build = "1"
```

### `crates/wf-db/Cargo.toml`

```toml
[dependencies]
sqlx        = { workspace = true }
tokio       = { workspace = true }
tokio-util  = { workspace = true }
serde       = { workspace = true }
thiserror   = { workspace = true }
uuid        = { workspace = true }
chrono      = { workspace = true }
anyhow      = { workspace = true }
tracing     = { workspace = true }
```

### `crates/wf-config/Cargo.toml`

```toml
[dependencies]
serde       = { workspace = true }
toml        = "0.8"
anyhow      = { workspace = true }
thiserror   = { workspace = true }
aes-gcm     = "0.10"
directories = "5"
uuid        = { workspace = true }
```

---

## 12. Testing Policy

| Target | Approach | Tooling |
|--------|----------|---------|
| `wf-db` service layer | Unit tests | SQLite in-memory |
| `wf-db` drivers (PG/MySQL) | Integration tests (`#[ignore]`) | Real DB |
| `wf-completion` | Unit tests | MetadataCache mock |
| `wf-config` ConfigManager | Unit tests | Temporary directory |
| `wf-config` encrypt/decrypt | Unit tests | — |
| `wf-query` analyzer/formatter | Unit tests | — |
| `wf-history` HistoryService | Unit tests | SQLite in-memory |

---

## 13. Logging

Uses the `tracing` crate with async span tracing support.

```rust
// Usage examples
tracing::info!(conn_id = %id, "Connected to database");
tracing::debug!(sql = %sql, duration_ms = %ms, "Query executed");
tracing::warn!("Metadata cache flush failed: {}", e);
```

---

## Change Log

| Date | Description |
|------|-------------|
| 2026-03-31 | Initial version |
| 2026-04-01 | Workspace structure update: app/ + crates/(wf-db, wf-config, wf-query, wf-completion, wf-history) |

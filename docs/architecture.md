# Wellfeather アーキテクチャ設計書

> 最終更新: 2026-03-31

---

## 1. 全体アーキテクチャ

```
┌─────────────────────────────────────────────────────┐
│                   UI Layer (Slint)                  │
│  sidebar.slint / editor.slint / result_table.slint  │
└──────────────────┬──────────────────────────────────┘
                   │  Slint callbacks + invoke_from_event_loop
┌──────────────────▼──────────────────────────────────┐
│               AppController                         │
│  Command受信 → Service呼び出し → Event送信          │
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

## 2. 通信パターン: Controller + Channel ハイブリッド

UI と バックエンドは **Command** と **Event** の2方向チャネルで通信する。
UI は Command を送るだけ、Controller は Event を返すだけ。UI更新は常に `invoke_from_event_loop` 経由。

```
UI (Slint)
  │  tx_cmd.send(Command::RunQuery(sql))
  │
  ▼
AppController (tokio task でループ)
  │  rx_cmd.recv() → match cmd → Service呼び出し
  │  tx_event.send(Event::QueryResult(...))
  │
  ▼
UI更新ハンドラ (invoke_from_event_loop)
  │  rx_event.recv() → match event → Slint property set
```

### Command / Event 定義

```rust
/// UI → Controller
pub enum Command {
    Connect(DbConnection),
    Disconnect(String),           // connection_id
    RunQuery(String),             // sql
    RunSelection(String),         // sql (選択範囲)
    RunAll(String),               // sql (全文)
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

## 3. 状態管理

### 設計方針
- `Arc<AppState>` を全サービスで共有（外側にはロックなし）
- 各サブ状態が内部に `RwLock<Data>` を持つ
- Controller / UI は直接 `RwLock` を触らない（メソッド経由のみ）
- 状態変更は `StateEvent` としてチャネルに流してUI更新を統一

### 構造

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

## 4. DB層の設計

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

> **発展方針**: 各DB固有の差分が増えた段階で、内部に `trait QueryExecutor` を導入し実装を分離する。

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

## 5. エラーハンドリング

### 方針
- `db / query` モジュール: `thiserror` で型付きエラー定義
- `Controller` 以上: `anyhow` でコンテキスト付きエラー処理

```rust
// db/error.rs
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

// controller.rs (anyhow使用)
pub async fn run_query(&self, sql: &str) -> anyhow::Result<()> {
    let result = self.db.execute(conn_id, sql)
        .await
        .context("Query execution failed")?;
    // ...
}
```

---

## 6. SQL補完の設計

```
入力イベント
  │
  ├─ debounce (300ms)
  │
  ▼
CompletionService.complete(sql, cursor_pos)
  │
  ├─ Parser: カーソル位置のコンテキスト解析
  │    - キーワード / テーブル名 / カラム名 を判定
  │
  ├─ MetadataCache (in-memory) から候補取得
  │    - テーブル一覧 / カラム一覧（FROM句連動）
  │
  └─ 結果: Vec<CompletionItem>
```

### MetadataCache

```rust
pub struct MetadataCache {
    memory: RwLock<HashMap<String, DbMetadata>>,  // conn_id → metadata
    db_path: PathBuf,                             // SQLite flush先
}

impl MetadataCache {
    pub async fn load(&self, conn_id: &str) -> Option<DbMetadata>;
    pub async fn store(&self, conn_id: &str, meta: DbMetadata);
    pub async fn flush_to_disk(&self) -> anyhow::Result<()>;  // 定期呼び出し
}
```

---

## 7. クエリキャンセルの設計

`tokio_util::sync::CancellationToken` を使用。

```rust
// クエリ実行時
let token = CancellationToken::new();
state.query.set_cancel_token(token.clone());

tokio::select! {
    result = db.execute(sql) => { /* 正常完了 */ }
    _ = token.cancelled()   => { /* キャンセル */ }
}

// Esc / Cancel ボタン時
state.query.cancel();  // → token.cancel() を内部で呼ぶ
```

---

## 8. 設定管理

```rust
pub struct ConfigManager {
    path: PathBuf,  // OS設定ディレクトリ / config.toml
}

impl ConfigManager {
    pub fn load() -> anyhow::Result<Config>;
    pub fn save(&self, config: &Config) -> anyhow::Result<()>;  // 即時保存
    pub fn app_dir() -> PathBuf {
        // directories クレートで解決
        // Windows: %APPDATA%\wellfeather
        // macOS:   ~/Library/Application Support/wellfeather
        // Linux:   ~/.config/wellfeather
    }
}
```

---

## 9. セッション復元

アプリ起動時のシーケンス:

```
1. ConfigManager::load() → Config（接続一覧 + 最後のセッション情報）
2. 最後のアクティブ接続IDに自動接続
3. 最後のクエリ文字列をエディタに復元
4. MetadataCache を SQLite から読み込み（バックグラウンド）
5. UI表示
```

---

## 10. 主要依存クレート

```toml
[dependencies]
slint            = "1"
sqlx             = { features = ["postgres", "mysql", "sqlite", "runtime-tokio"] }
tokio            = { features = ["full"] }
tokio-util       = "0.7"      # CancellationToken
serde            = { features = ["derive"] }
toml             = "0.8"
anyhow           = "1"
thiserror        = "1"
tracing          = "0.1"
tracing-subscriber = "0.3"
aes-gcm          = "0.10"     # パスワード暗号化
directories      = "5"        # OS設定ディレクトリ解決
uuid             = { features = ["v4"] }
chrono           = "0.4"
```

---

## 11. ディレクトリ構成（確定版）

```
src/
├── main.rs                  # Slint初期化・Controller起動・チャネル接続
│
├── ui/
│   ├── app.slint
│   └── components/
│       ├── sidebar.slint
│       ├── editor.slint
│       ├── result_table.slint
│       └── status_bar.slint
│
├── app/
│   ├── controller.rs        # Command受信・Service呼び出し・Event送信
│   ├── command.rs           # Command enum
│   ├── event.rs             # Event / StateEvent enum
│   └── session.rs           # セッション復元ロジック
│
├── state/
│   ├── mod.rs               # AppState / SharedState
│   ├── connection_state.rs  # ConnectionState + ConnectionData
│   ├── query_state.rs       # QueryState + QueryData
│   └── ui_state.rs          # UiState + UiData
│
├── db/
│   ├── mod.rs
│   ├── service.rs           # DbService
│   ├── pool.rs              # DbPool enum / DbKind enum
│   ├── error.rs             # DbError (thiserror)
│   ├── models.rs            # QueryResult / DbMetadata / DbConnection
│   └── drivers/
│       ├── pg.rs            # PostgreSQL固有実装
│       ├── my.rs            # MySQL固有実装
│       └── sqlite.rs        # SQLite固有実装
│
├── history/
│   ├── mod.rs
│   └── service.rs           # HistoryService (SQLite)
│
├── completion/
│   ├── mod.rs
│   ├── service.rs           # CompletionService
│   ├── cache.rs             # MetadataCache (Memory + SQLite flush)
│   └── parser.rs            # カーソル位置解析
│
├── config/
│   ├── mod.rs
│   ├── manager.rs           # ConfigManager (即時保存)
│   └── models.rs            # Config / FontConfig 構造体
│
└── crypto/
    └── mod.rs               # AES-256-GCM パスワード暗号化
```

---

## 12. テスト方針

| 対象 | 方式 | 使用するもの |
|------|------|-------------|
| Service層ロジック | ユニットテスト | モック実装 |
| DB操作 | 統合テスト | SQLite（インメモリ） |
| CompletionService | ユニットテスト | MetadataCache モック |
| ConfigManager | ユニットテスト | 一時ディレクトリ |
| 暗号化/復号 | ユニットテスト | - |

---

## 13. ロギング

`tracing` クレートを使用。非同期スパントレース対応。

```rust
// 使用例
tracing::info!(conn_id = %id, "Connected to database");
tracing::debug!(sql = %sql, duration_ms = %ms, "Query executed");
tracing::warn!("Metadata cache flush failed: {}", e);
```

---

## 変更履歴

| 日付 | 内容 |
|------|------|
| 2026-03-31 | 初版作成 |

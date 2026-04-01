# Wellfeather 仕様書

> 最終更新: 2026-03-31
>
> **コアバリュー**: 軽い・速い・キーボード中心・クエリ体験
> **競合との差別化**: DBeaver / TablePlus / DataGrip より起動が速く、メモリが軽く、キーボード操作が優れている

---

## 1. プロジェクト概要

| 項目 | 内容 |
|------|------|
| アプリ名 | `wellfeather` |
| 対象プラットフォーム | Windows / macOS / Linux（クロスプラットフォーム） |
| 目標: 起動時間 | < 1秒 |
| 目標: メモリ | 最小化 |
| UI応答 | ノンブロッキング（全非同期） |

### 競合比較

| | DBeaver | TablePlus | DataGrip | **wellfeather** |
|--|---------|-----------|----------|-----------------|
| 起動時間 | 10〜30秒 | 2〜3秒 | 10〜20秒 | **< 1秒** |
| メモリ | 500MB〜 | 150MB〜 | 400MB〜 | **最小化** |
| キーボード操作 | △ | △ | ○ | **◎** |
| Vimキーバインド | △(plugin) | × | △ | Phase 2 |
| コマンドパレット | × | × | ○ | Phase 2 |
| SQL補完 | ○ | △ | ◎ | 軽量版(MVP) |
| SQLフォーマッター | △ | △ | ◎ | **◎(MVP)** |
| 仮想スクロール | △ | ○ | ○ | **◎(MVP)** |
| NULL視認性 | △ | ○ | ○ | **◎(MVP)** |
| テーブルDBクリック→SELECT挿入 | × | ○ | × | **○(MVP)** |

---

## 2. 技術スタック

| 項目 | 技術 |
|------|------|
| 言語 | Rust |
| UI | Slint |
| DB接続 | sqlx |
| 非同期 | tokio |
| 対応DB | PostgreSQL / MySQL / SQLite |

---

## 3. アーキテクチャ

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

### 方針
- UIは状態表示に徹する
- ロジックはRust側に集約
- 非同期処理はUIから分離
- `slint::invoke_from_event_loop` でUI更新

---

## 4. ディレクトリ構成（暫定）

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
│   └── session.rs          // セッション復元
├── db/
│   ├── connection.rs
│   ├── query_executor.rs
│   └── models.rs
├── query/
│   ├── history.rs          // SQLite永続化
│   ├── formatter.rs        // SQLフォーマッター
│   └── analyzer.rs         // カーソル位置解析（補完用）
└── completion/
    ├── metadata_provider.rs
    └── engine.rs
```

---

## 5. UI レイアウト

```
+---------------------------------------------------------------+
| メニューバー                                                   |
+-------------------+-------------------------------------------+
| Sidebar           | Query Editor                              |
|                   |  行番号 | SQL入力エリア                   |
| ▼ my_postgres     |                                           |
|   ▼ Tables        +-------------------------------------------+
|     ▶ users       | Result Table（仮想スクロール）             |
|     ▶ orders      |  NULL: バッジ表示                         |
|   ▼ Views         |  コピー: Ctrl+C / 右クリックメニュー      |
|     ▶ active_...  +-------------------------------------------+
|   ▶ Stored Procs  | 下部プレビューペイン（長テキスト展開）     |
|   ▶ Indexes       |                                           |
+-------------------+-------------------------------------------+
| Status Bar: [接続名/DB] [実行時間] [行数] [エラー/成功]       |
+---------------------------------------------------------------+
```

---

## 6. 接続管理

### 永続化

| 項目 | 内容 |
|------|------|
| 設定ファイル形式 | TOML |
| 保存場所 | OS標準設定ディレクトリ |
| &nbsp;&nbsp;Windows | `%APPDATA%\wellfeather\` |
| &nbsp;&nbsp;macOS | `~/Library/Application Support/wellfeather/` |
| &nbsp;&nbsp;Linux | `~/.config/wellfeather/` |
| ファイル構成 | `config.toml`（接続設定 / アプリ設定） |
| 履歴DB | `history.db`（SQLite、同ディレクトリ） |

### 接続入力方式
両方対応（ユーザーが選択できる）:
- 接続文字列: `postgres://user:pass@host:5432/dbname`
- 個別フィールド: ホスト / ポート / ユーザー / パスワード / DB名

### パスワード保存
- **方式**: AES-256-GCM で暗号化し `config.toml` に保存
- アプリ固有鍵の管理方法は実装フェーズで詳細化

### データ構造

```rust
pub enum DbType {
    PostgreSQL,
    MySQL,
    SQLite,
}

pub struct DbConnection {
    pub id: String,                          // UUID
    pub name: String,                        // 表示名
    pub db_type: DbType,
    // 接続文字列モード
    pub connection_string: Option<String>,
    // 個別フィールドモード
    pub host: Option<String>,
    pub port: Option<u16>,
    pub user: Option<String>,
    pub password_encrypted: Option<String>,  // AES-256-GCM 暗号化済み
    pub database: Option<String>,
}
```

### 複数接続
- 複数DBへの同時接続を保持
- サイドバーで全接続を一覧表示、アクティブ接続を切り替え

---

## 7. セッション復元

起動時に前回の状態を自動復元する:
- 最後にアクティブだった接続に自動接続
- 最後のクエリ文字列をエディタに復元
- 接続情報は `config.toml` のセッションセクションに保存

---

## 8. サイドバー

### 表示項目（ツリー構造）

```
▼ [接続名: my_postgres]
  ▼ Tables
    ▶ users
    ▶ orders
  ▼ Views
    ▶ active_users
  ▼ Stored Procedures
    ▶ get_user_by_id
  ▼ Indexes
    ▶ users_pkey
▼ [接続名: local_sqlite]
  ...
```

- 各ノードは折りたたみ・展開可能
- 全項目（Tables / Views / Stored Procedures / Indexes）を表示

### インタラクション
- テーブル名を**ダブルクリック** → エディタに `SELECT * FROM テーブル名` を挿入

---

## 9. SQLエディタ

| 項目 | 内容 |
|------|------|
| 行番号 | 表示（MVP） |
| シンタックスハイライト | Phase 2 |
| マルチタブ | Phase 2 |
| Vimキーバインド | Phase 2 |
| SQL補完トリガー | 自動ポップアップ + `Ctrl+Space` 両方 |
| SQLフォーマッター | `Ctrl+Shift+F` で整形（MVP） |
| フォント設定 | `config.toml` で変更可能（MVP） |

### キーボードショートカット（確定）

| キー | 動作 |
|------|------|
| `Ctrl+Enter` | カーソルがある SQL 文のみ実行 |
| `Ctrl+Shift+Enter` | エディタ全文を実行 |
| `Shift+Enter` | 選択範囲のみ実行 |
| `Ctrl+Space` | 補完候補を手動表示 |
| `Ctrl+Shift+F` | SQL を整形（フォーマッター） |
| `Esc` | 実行中クエリをキャンセル |
| `Alt+↑/↓/←/→` | ペイン間移動 |

---

## 10. SQL補完

### 方針
- 軽量・高速を優先（重いLSP統合はPhase 2以降）
- 接続時にDBメタデータをローカルメモリキャッシュ

### 構成
- **Metadata Provider**: 接続時にテーブル / カラム / ビュー / インデックス情報を取得
- **Completion Engine**:
  - SQLキーワード補完（`SELECT`, `WHERE` 等）
  - テーブル名補完
  - カラム補完（`FROM` 句に応じて絞り込み）
- **Parser（簡易）**: カーソル位置のコンテキスト解析

### トリガー
- 入力中に自動ポップアップ
- `Ctrl+Space` でも手動呼び出し可能

---

## 11. クエリ実行

### 実行フロー

```
1. UIからrun_query呼び出し（Ctrl+Enter / Ctrl+Shift+Enter / Shift+Enter）
2. Controllerが受け取り実行開始
3. tokioタスクで非同期実行
4. 結果をAppStateに反映
5. slint::invoke_from_event_loop でUI更新
6. ステータスバーに実行時間・行数を表示
```

### クエリキャンセル
- **MVP**: 実行中に `Esc` キーまたは「Cancel」ボタンで中断
- **Phase 2**: タイムアウト設定（例: 3分経過で自動キャンセル）

### データ構造

```rust
pub async fn execute_query(sql: &str, conn: &DbConnection) -> Result<QueryResult>

pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,  // NULLはNone
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

## 12. 結果テーブル

| 項目 | 内容 |
|------|------|
| レンダリング | 仮想スクロール（表示範囲のみ描画、大量行対応） |
| ページネーション | ユーザー選択式（100 / 500 / 1000 行） |
| カラムソート | クライアントサイドソート（MVP） |
| NULL表示 | バッジ/ピルUI（小さく `NULL` と表示、視認性高） |
| データ編集 | 読み取り専用（MVP） |
| エクスポート | CSV + JSON（MVP） |
| コピー | `Ctrl+C`: セル値、右クリックメニュー: セル値 / 行全体 / TSV形式 |
| 下部プレビューペイン | 選択セルの全文を常時表示（長テキスト・JSON対応） |

---

## 13. エラー表示

- **方式**: 結果エリアにインライン表示（ダイアログ不使用）
- ステータスバーにもエラー要約を表示

---

## 14. ステータスバー

左から順に以下を表示:

```
[ 接続名: my_postgres / dbname ] [ 42 ms ] [ 128 行 ] [ ✓ 実行成功 / ✗ エラーメッセージ ]
```

---

## 15. クエリ履歴

| 項目 | 内容 |
|------|------|
| 保存先 | SQLite（`history.db`、OS設定ディレクトリ） |
| 最大保持件数 | 無制限 |
| 検索・フィルタ | Phase 2 |
| 表示UI | Phase 2 |

---

## 16. アプリ設定（`config.toml`）

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
    pub cancel_token: Option<CancellationToken>, // クエリキャンセル用
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

## 18. MVPフィーチャースコープ

### MVP（Phase 1）に含む

| 機能 | 備考 |
|------|------|
| DB接続（Postgres / MySQL / SQLite） | |
| 接続情報の保存（TOML + AES-256-GCM暗号化） | |
| 複数接続同時保持・切り替え | |
| セッション復元（前回の接続・クエリ） | |
| サイドバー（ツリー構造） | Tables / Views / Stored Procs / Indexes |
| テーブルダブルクリック → `SELECT * FROM` 挿入 | |
| SQLエディタ（行番号あり） | |
| クエリ実行（カーソル文 / 全文 / 選択） | |
| クエリキャンセル（Esc / Cancelボタン） | |
| SQL補完（軽量メタデータベース） | |
| SQLフォーマッター（Ctrl+Shift+F） | |
| 結果テーブル（仮想スクロール） | |
| NULL バッジ表示 | |
| カラムソート（クライアントサイド） | |
| 下部プレビューペイン（長テキスト展開） | |
| コピー（セル値 / 行 / TSV） | |
| エクスポート（CSV + JSON） | |
| ステータスバー | |
| エラーのインライン表示 | |
| クエリ履歴保存（SQLite） | |
| テーマ切り替え（ダーク/ライト） | |
| フォント設定（config.toml） | |

### Phase 2

| 機能 |
|------|
| シンタックスハイライト |
| マルチタブ |
| Vimキーバインド |
| コマンドパレット |
| 履歴検索・フィルタUI |
| 遅いクエリ検出 |
| EXPLAIN表示 |
| クエリタイムアウト設定（自動キャンセル） |
| 履歴検索UI |

### Phase 3

| 機能 |
|------|
| 軽量分析・パフォーマンスヒント |
| インデックス提案 |
| OSキーチェーン連携 |
| CLI連携 |
| LSPベースSQL補完 |

---

## 19. MVP対象外（意図的に除外）

- 高度なメトリクス
- 複雑なプラグイン
- 重いUI
- セル編集・UPDATE発行
- ダイアログ型エラー表示

---

## 20. 非機能要件

| 項目 | 要件 |
|------|------|
| 起動時間 | < 1秒 |
| メモリ使用量 | 最小化 |
| UI応答性 | ノンブロッキング |
| 大量データ | 仮想スクロールで数万行対応 |

---

## 21. ローカライズ（i18n）

### 対応言語

| コード | 言語 |
|--------|------|
| `en` | English（デフォルト） |
| `ja` | 日本語 |

言語設定は `config.toml` の `[ui] language` フィールドで指定する。起動時に読み込み、アプリ全体に適用する。

### ハイブリッド i18n アーキテクチャ

UI 文字列と Rust 側メッセージで異なるツールを使う：

| 対象 | ツール | ファイル形式 | マクロ |
|------|--------|-------------|--------|
| `.slint` 内の UI 文字列 | Slint 組み込み i18n (GNU gettext) | `.po` | `@tr("key")` |
| Rust 側メッセージ（エラー・ライフサイクル） | `rust-i18n` crate | `.yml` | `t!("ns.key")` |

### ファイル構成

```
app/
├── lang/
│   ├── en/LC_MESSAGES/wellfeather.po   ← Slint UI 文字列（英語）
│   └── ja/LC_MESSAGES/wellfeather.po   ← Slint UI 文字列（日本語）
└── locales/
    ├── en.yml                           ← Rust 側メッセージ（英語）
    └── ja.yml                           ← Rust 側メッセージ（日本語）
```

### 初期化

```rust
// app/src/main.rs または lib.rs
slint::init_translations!(concat!(env!("CARGO_MANIFEST_DIR"), "/lang"));
slint::select_bundled_translation(&config.ui.language);

rust_i18n::i18n!("locales", fallback = "en");
rust_i18n::set_locale(&config.ui.language);
```

### `.yml` ネーミング規則

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

### `LocalizedMessage` トレイト

UI に表示するエラーはすべて `LocalizedMessage` トレイトを実装し、`t!()` で翻訳済み文字列を返す。

```rust
pub trait LocalizedMessage {
    fn localized_message(&self) -> String;
}
```

`wf-db`、`wf-config`、`wf-query`、`wf-completion`、`wf-history` の各エラー型がこのトレイトを実装する。

### `.slint` 文字列規則

`.slint` ファイル内のユーザー向け文字列は必ず `@tr("key")` を使用する。ハードコードされた英語文字列は禁止。

---

## 変更履歴

| 日付 | 内容 |
|------|------|
| 2026-03-31 | 初版: 基本仕様決定 |
| 2026-03-31 | 競合比較を踏まえた詳細仕様追加（仮想スクロール・NULL表示・フォーマッター等） |
| 2026-04-01 | §21 ローカライズ仕様追加（Slint i18n + rust-i18n ハイブリッド、en/ja 対応） |

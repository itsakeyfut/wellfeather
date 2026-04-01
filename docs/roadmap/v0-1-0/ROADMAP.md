# v0.1.0 — プロジェクト基盤

> **テーマ**: 土台を作る。アプリが起動する状態にする。
> **前提バージョン**: なし（初回）

---

## 目標

コードを書き始めるための基盤を整える。
依存クレートの設定・コアデータモデル・設定管理・暗号化・状態管理・Slint UIシェルを揃え、
**アプリが起動し空のレイアウトが表示される**状態にする。

---

## 達成基準 (Exit Criteria)

- [ ] `cargo build --release` が通る
- [ ] アプリを起動すると空のウィンドウ（サイドバー / エディタ / 結果エリア / ステータスバー のレイアウト）が表示される
- [ ] 起動時間が < 1秒（リリースビルド）
- [ ] `cargo test` が通る（config・crypto のユニットテスト）
- [ ] OS設定ディレクトリに `config.toml` が生成される

---

## 対象機能・実装範囲

| カテゴリ | 内容 |
|---------|------|
| プロジェクト設定 | Cargo.toml（依存クレート定義）、build.rs（Slintコンパイル） |
| ディレクトリ構造 | architecture.md 記載の src/ 以下全モジュール骨格 |
| 設定管理 | config/models.rs（Config構造体）、config/manager.rs（TOML読み書き） |
| 暗号化 | crypto/mod.rs（AES-256-GCM 暗号化・復号） |
| データモデル | db/models.rs（DbConnection, QueryResult等）、db/error.rs |
| 状態管理 | state/（AppState, ConnectionState, QueryState, UiState） |
| Command/Event | app/command.rs、app/event.rs |
| UI シェル | ui/app.slint（空レイアウト）、main.rs（Slint+tokio起動） |

---

## 実装しないもの（スコープ外）

- DB への実際の接続
- クエリ実行
- UI コンポーネントの機能実装（レイアウト骨格のみ）

---

## 主要リスク・注意点

- Slint の build.rs 設定は最初に正確に行う。後から変更するとコンパイルエラーが追いにくい
- `slint::include_modules!()` と Slint global の命名に注意（コンパイル生成コードとの整合）
- アプリ固有の暗号化鍵（AES master key）をどこに保存するか最初に決める
  - MVP: OS設定ディレクトリに `.key` ファイルとして保存（初回起動時に生成）
- Windows / macOS / Linux で `directories` クレートのパスが異なることを考慮

---

## タスク一覧

詳細は `docs/roadmap/tasks/v0-1-0.md` を参照。

| タスクID | タイトル |
|---------|---------|
| T011 | Cargo.toml & build.rs 初期設定 |
| T012 | ディレクトリ構造・モジュール骨格作成 |
| T013 | config/models.rs — 設定モデル定義 |
| T014 | config/manager.rs — 設定ファイル管理 |
| T015 | crypto/mod.rs — AES-256-GCM 暗号化 |
| T016 | db/models.rs + db/error.rs — DBデータモデル |
| T017 | state/ — 状態管理実装 |
| T018 | app/command.rs + app/event.rs — Command/Event定義 |
| T019 | UIシェル — main.rs + ui/app.slint 基本レイアウト |

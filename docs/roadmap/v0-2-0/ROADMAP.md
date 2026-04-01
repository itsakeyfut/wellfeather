# v0.2.0 — DB接続管理

> **テーマ**: 実際のDBに接続する。接続情報を安全に管理する。
> **前提バージョン**: v0.1.0

---

## 目標

PostgreSQL / MySQL / SQLite への接続・切断・切り替えができる状態にする。
接続情報は TOML + AES-256-GCM で永続化し、次回起動時に自動復元する。

---

## 達成基準 (Exit Criteria)

- [ ] UIから接続情報を追加し、DBへ接続できる（SQLite / PostgreSQL / MySQL）
- [ ] 複数の接続を同時に保持し、切り替えられる
- [ ] 接続情報が `config.toml` に暗号化保存される
- [ ] アプリ再起動後に前回の接続が自動復元される
- [ ] ステータスバーに接続名・DB名が表示される
- [ ] 統合テスト: SQLite接続のテストが通る

---

## 対象機能・実装範囲

| カテゴリ | 内容 |
|---------|------|
| DB接続層 | db/pool.rs（DbPool enum）、db/drivers/sqlite.rs, pg.rs, my.rs（接続のみ） |
| DbService | db/service.rs — connect / disconnect / pools HashMap管理 |
| Controller | app/controller.rs — Command::Connect / Disconnect 処理、Eventループ開始 |
| 接続管理UI | 接続追加ダイアログ（個別フィールド + 接続文字列の両対応） |
| 接続一覧UI | サイドバー上部の接続リスト、アクティブ接続の切り替え |
| セッション | app/session.rs — 前回接続の保存・起動時自動接続 |
| ステータスバー | 接続名 / DB名 の表示 |

---

## 実装しないもの（スコープ外）

- クエリ実行（接続確立のみ）
- メタデータ取得（v0.4.0）
- 接続のサイドバーツリー展開（v0.4.0）

---

## 主要リスク・注意点

- sqlx の `AnyPool` は使わず `DbPool` enum dispatch を採用（architecture.md 参照）
- 接続タイムアウト・接続失敗時のリトライは MVP では考慮しない（エラー表示のみ）
- MySQL / PostgreSQL の統合テストには実際のDBが必要なため、CI環境での扱いに注意
  - ローカルの SQLite 統合テストを優先し、PG/MySQL はオプション扱いにする
- パスワードの暗号化鍵（crypto::key()）は v0.1.0 で確立したものを使う

---

## タスク一覧

詳細は `docs/roadmap/tasks/v0-2-0.md` を参照。

| タスクID | タイトル |
|---------|---------|
| T021 | db/pool.rs — DbPool enum + 接続関数 |
| T022 | db/drivers/sqlite.rs — SQLite接続実装 |
| T023 | db/drivers/pg.rs — PostgreSQL接続実装 |
| T024 | db/drivers/my.rs — MySQL接続実装 |
| T025 | db/service.rs — DbService（connect/disconnect） |
| T026 | app/controller.rs — Commandループ + Connect/Disconnect処理 |
| T027 | 接続管理UI — 追加ダイアログ + 一覧・切り替え |
| T028 | app/session.rs — セッション保存・復元 |
| T029 | status_bar.slint — 接続名/DB名表示 |

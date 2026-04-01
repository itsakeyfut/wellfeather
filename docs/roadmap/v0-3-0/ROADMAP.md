# v0.3.0 — クエリ実行コア

> **テーマ**: SQL を書いて実行し、結果を見る。コアバリューの中核。
> **前提バージョン**: v0.2.0

---

## 目標

SQLエディタにクエリを書き、実行し、結果を確認できる状態にする。
これが wellfeather の最も重要な体験であり、本バージョンで「使えるツール」になる。

---

## 達成基準 (Exit Criteria)

- [ ] SQLエディタに複数行のSQLを入力できる（行番号表示あり）
- [ ] Ctrl+Enter でカーソル位置のSQL文を実行できる
- [ ] Ctrl+Shift+Enter でエディタ全文を実行できる
- [ ] Shift+Enter で選択範囲のSQLを実行できる
- [ ] 結果テーブルにカラムヘッダーと行データが表示される
- [ ] クエリ実行中に Esc / Cancel でキャンセルできる
- [ ] エラー時は結果エリアにインラインでエラーメッセージが表示される
- [ ] ステータスバーに実行時間（ms）と結果行数が表示される
- [ ] クエリ実行のたびに `history.db` に記録される
- [ ] UI がクエリ実行中もフリーズしない（ノンブロッキング）

---

## 対象機能・実装範囲

| カテゴリ | 内容 |
|---------|------|
| SQLエディタ | editor.slint — multiline テキストエリア + 行番号 |
| キーショートカット | Ctrl+Enter / Ctrl+Shift+Enter / Shift+Enter / Esc |
| DB実行層 | db/drivers/ — execute 実装（全DB）、CancellationToken対応 |
| DbService | execute / execute_with_cancel |
| Controller | Command::RunQuery / RunSelection / RunAll / CancelQuery 処理 |
| 非同期フロー | tokio::spawn + invoke_from_event_loop によるUI更新 |
| 結果テーブル | result_table.slint — 基本テーブル（カラム + 行、VecModel） |
| エラー表示 | 結果エリアへのインライン表示 |
| ステータスバー | 実行時間・行数の更新 |
| クエリ履歴 | history/service.rs — SQLite に実行ログ保存 |

---

## 実装しないもの（スコープ外）

- 仮想スクロール（v0.5.0）
- NULL バッジ・カラムソート（v0.5.0）
- SQL補完・フォーマッター（v0.6.0）
- サイドバーのメタデータ表示（v0.4.0）

---

## 主要リスク・注意点

- **invoke_from_event_loop** のクロージャ内では `Rc` が使えない。VecModel 生成はクロージャ内で行う（reference-patterns.md 参照）
- Slint の TextInput は multiline モードでキーイベントのカスタマイズに制限がある場合がある。Ctrl+Enter 等のキーバインド実装に注意
- カーソル位置の SQL 文抽出ロジック（`;` 区切り）は query/analyzer.rs に切り出す
- `tokio::select!` によるキャンセルは DB ドライバによって実際の中断タイミングが異なる。sqlx はクエリキャンセルに `cancel_token` を直接サポートしない場合があり、接続自体を drop する方法を取ることがある

---

## タスク一覧

詳細は `docs/roadmap/tasks/v0-3-0.md` を参照。

| タスクID | タイトル |
|---------|---------|
| T031 | editor.slint — SQLエディタ基本実装（行番号あり） |
| T032 | query/analyzer.rs — カーソル位置からSQL文抽出 |
| T033 | db/drivers/ — execute 実装（SQLite・PG・MySQL） |
| T034 | db/service.rs — execute / execute_with_cancel |
| T035 | app/controller.rs — RunQuery/Cancel コマンド処理 |
| T036 | result_table.slint — 基本結果テーブル |
| T037 | エラーインライン表示 |
| T038 | status_bar.slint — 実行時間・行数更新 |
| T039 | history/service.rs — HistoryService（SQLite保存） |

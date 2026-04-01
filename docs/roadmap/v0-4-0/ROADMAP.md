# v0.4.0 — スキーマブラウザ

> **テーマ**: DB構造を視覚的に把握し、テーブルからクエリを素早く始める。
> **前提バージョン**: v0.3.0

---

## 目標

サイドバーに接続DBのスキーマツリーを表示する。
テーブル名をダブルクリックするだけで `SELECT * FROM テーブル名` がエディタに挿入され、
即座にクエリを試せる体験を実現する。

---

## 達成基準 (Exit Criteria)

- [ ] 接続後にサイドバーにスキーマツリーが表示される
  - Tables / Views / Stored Procedures / Indexes の4カテゴリ
- [ ] 各ノードを折りたたみ・展開できる
- [ ] テーブル名をダブルクリックすると `SELECT * FROM テーブル名` がエディタに挿入される
- [ ] メタデータは接続時にバックグラウンドで非同期取得される（UI がフリーズしない）
- [ ] 取得したメタデータは MetadataCache（メモリ + SQLite）に保存される
- [ ] Alt+↑/↓/←/→ でサイドバー・エディタ・結果テーブル間のフォーカスを移動できる

---

## 対象機能・実装範囲

| カテゴリ | 内容 |
|---------|------|
| メタデータ取得 | db/drivers/ — fetch_metadata 実装（全DB） |
| MetadataCache | completion/cache.rs — メモリ + SQLite flush |
| Controller | 接続完了後に fetch_metadata を非同期実行、Event::MetadataLoaded 送信 |
| サイドバーUI | sidebar.slint — ツリー構造、折りたたみ/展開 |
| テーブルダブルクリック | サイドバー → エディタへの SELECT * FROM 挿入 |
| ペイン移動 | Alt+Arrow フォーカス移動 |

---

## 実装しないもの（スコープ外）

- テーブルの DDL 表示・カラム詳細表示（将来）
- メタデータの手動更新ボタン（将来）
- インデックス・ストアドプロシージャの詳細表示（将来）

---

## 主要リスク・注意点

- **Slint のツリービュー実装**: Slint には標準のツリービューコンポーネントがない。ListView のネストや条件分岐で実装する必要がある
- 各DBのメタデータ取得SQLが異なるため、drivers ごとに実装を分ける
  - PostgreSQL: `information_schema.tables / columns`
  - MySQL: `information_schema.tables / columns`
  - SQLite: `sqlite_master` テーブル（テーブルのみ、views/procs/indexesは制限あり）
- メタデータ量が多い場合（数百テーブル）のサイドバー描画パフォーマンスに注意
- Alt+Arrow がOSやSlintのデフォルトキーバインドと衝突する可能性を確認する

---

## タスク一覧

詳細は `docs/roadmap/tasks/v0-4-0.md` を参照。

| タスクID | タイトル |
|---------|---------|
| T041 | db/drivers/ — fetch_metadata 実装（SQLite・PG・MySQL） |
| T042 | completion/cache.rs — MetadataCache（メモリ + SQLite flush） |
| T043 | app/controller.rs — MetadataLoaded Event処理 |
| T044 | sidebar.slint — ツリー構造UI（折りたたみ/展開） |
| T045 | テーブルダブルクリック → SELECT * FROM 挿入 |
| T046 | Alt+Arrow ペイン移動 |

# v0.6.0 — SQL体験

> **テーマ**: SQL を書く体験を DataGrip レベルに近づける。軽量・高速で。
> **前提バージョン**: v0.5.0

---

## 目標

SQL補完とフォーマッターを実装し、「書きやすく・読みやすい」クエリ体験を実現する。
補完は DBeaver / TablePlus より優れた体験を、フォーマッターは DataGrip に匹敵するものを、
いずれも軽量・高速に実装する。

---

## 達成基準 (Exit Criteria)

- [ ] 入力から 300ms 後に補完候補が自動表示される
- [ ] Ctrl+Space で手動補完トリガーができる
- [ ] SQLキーワード（SELECT, WHERE, JOIN等）が補完される
- [ ] 接続中のDBのテーブル名が補完される
- [ ] FROM句に応じたカラム名補完が働く
- [ ] 補完候補を ↑↓/Enter で選択・挿入できる、Esc でキャンセルできる
- [ ] Ctrl+Shift+F で SQL が整形される（インデント・キーワード大文字化）
- [ ] 補完・フォーマッターどちらも < 100ms で応答する

---

## 対象機能・実装範囲

| カテゴリ | 内容 |
|---------|------|
| パーサー | completion/parser.rs — カーソル位置のコンテキスト判定 |
| 補完エンジン | completion/engine.rs — キーワード/テーブル/カラム候補生成 |
| CompletionService | completion/service.rs — debounce 300ms + Ctrl+Space |
| 補完UI | Slint ポップアップ（候補リスト、キーボード選択） |
| SQLフォーマッター | query/formatter.rs — 軽量整形ロジック |
| ショートカット | Ctrl+Shift+F フォーマッター実行 |

---

## 実装しないもの（スコープ外）

- LSP（sql-language-server）統合（Phase 3）
- 型推論ベース補完（Phase 3）
- シンタックスハイライト（v1.1.0）
- エイリアスを考慮したカラム補完（将来）

---

## 主要リスク・注意点

- **Slint でのポップアップ実装**: 補完候補をエディタ上に重ねて表示するポップアップは Slint の PopupWindow で実現できるが、カーソル位置への配置は工夫が必要
- Slint の TextInput はカーソル位置（文字インデックス）取得の API が限定的な場合がある。実装前に Slint のカーソル位置取得方法を確認する
- フォーマッターは外部クレート（`sqlformat` 等）を使うか自前実装かを判断する
  - 軽量優先のため `sqlformat` クレート（Rust実装、軽量）の採用を検討する
- 補完の debounce は `slint::Timer::SingleShot` で実装（reference-patterns.md 参照）

---

## タスク一覧

詳細は `docs/roadmap/tasks/v0-6-0.md` を参照。

| タスクID | タイトル |
|---------|---------|
| T061 | completion/parser.rs — カーソル位置コンテキスト解析 |
| T062 | completion/engine.rs — 補完候補生成 |
| T063 | completion/service.rs — CompletionService（debounce） |
| T064 | 補完ポップアップUI |
| T065 | query/formatter.rs — SQLフォーマッター |

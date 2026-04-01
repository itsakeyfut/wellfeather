# v1.4.0 — EXPLAIN表示

> **テーマ**: クエリのパフォーマンスを視覚的に理解できるようにする。
> **前提バージョン**: v1.3.0

---

## 目標

EXPLAIN / EXPLAIN ANALYZE の結果をツリー構造で表示し、
クエリのボトルネックを一目で把握できるようにする。

---

## 達成基準 (Exit Criteria)

- [ ] 「EXPLAIN」ボタンまたはショートカットでクエリのEXPLAINが実行される
- [ ] DBごとに適切なEXPLAINコマンドが使われる（PG: EXPLAIN ANALYZE, MySQL: EXPLAIN, SQLite: EXPLAIN QUERY PLAN）
- [ ] 結果がノード単位のツリー表示になる
- [ ] 各ノードにコスト・実行時間（PostgreSQL EXPLAIN ANALYZE の場合）が表示される

---

## タスク一覧（概要）

詳細は `docs/roadmap/tasks/v1-4-0.md` を参照。

- EXPLAIN実行（DbPool::explain_prefix() 活用）
- EXPLAINパーサー（DBごとの出力を構造化）
- EXPLAINツリーUI（Slint）

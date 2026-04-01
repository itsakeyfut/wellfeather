# v2.1.0 — CLI連携・LSP補完

> **テーマ**: GUIとCLIのハイブリッド運用と、最高品質のSQL補完を実現する。
> **前提バージョン**: v2.0.0

---

## 目標

CLIからのクエリ実行とLSPベースの高精度SQL補完を実装し、
開発者の多様なワークフローに対応する。

---

## 達成基準 (Exit Criteria)

- [ ] `wellfeather query "SELECT * FROM users"` でCLIからクエリ実行できる
- [ ] CLIとGUIで接続設定・クエリエンジンを共有できる
- [ ] LSP（sql-language-server 等）ベースの補完が動作する

---

## タスク一覧（概要）

詳細は `docs/roadmap/tasks/v2-1-0.md` を参照。

- CLIサブコマンド（clap クレート）
- Query Engine の CLI / GUI 共有
- LSP統合（sql-language-server プロセス管理）

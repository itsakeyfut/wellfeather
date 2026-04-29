# v0.11.0 — EXPLAIN Visualization

> **Theme**: Understand query performance visually.
> **Prerequisite**: v0.10.0

---

## Goal

Display EXPLAIN / EXPLAIN ANALYZE results as a tree structure so users can
identify query bottlenecks at a glance.

---

## Exit Criteria

- [ ] An "EXPLAIN" button or keyboard shortcut runs EXPLAIN for the current query
- [ ] The appropriate EXPLAIN command is used per DB (PostgreSQL: EXPLAIN ANALYZE, MySQL: EXPLAIN, SQLite: EXPLAIN QUERY PLAN)
- [ ] Results are shown as a per-node tree
- [ ] Each node displays its cost and actual execution time (where available, e.g. PostgreSQL EXPLAIN ANALYZE)
- [ ] High-cost nodes are visually highlighted (red text or background)

---

## Task List

See `docs/roadmap/tasks/v0-11-0.md` for details.

| Task ID | Title | Issue |
|---------|-------|-------|
| T141 | EXPLAIN execution per DB type (PG, MySQL, SQLite) | #69 |
| T142 | EXPLAIN output parser: raw output to ExplainNode tree | #70 |
| T143 | EXPLAIN tree view UI | #71 |

# v0.15.0 — SQL Completion Enhancement

> **Theme**: Make the completion engine smarter and more context-aware.
> **Prerequisite**: v0.7.0

---

## Goal

Extend the SQL completion engine beyond basic keyword/table/column completion with
context-aware suggestions, fuzzy matching, snippets, and richer metadata display.

---

## Exit Criteria

- [ ] Multi-table JOIN column disambiguation works correctly
- [ ] INSERT and UPDATE statement column completion is supported
- [ ] Subquery context is detected and completion candidates are scoped accordingly
- [ ] Schema-qualified table names (e.g. `public.users`) are completed for PostgreSQL
- [ ] SQL built-in functions and aggregates (COUNT, SUM, NOW, etc.) are suggested
- [ ] Common SQL clause snippets (SELECT…FROM, JOIN…ON, etc.) are available
- [ ] Completion candidates can be ranked by query history frequency
- [ ] Fuzzy matching allows typing partial/approximate prefixes to find candidates
- [ ] Column type information is displayed in the completion item detail
- [ ] Keyword documentation is shown in a popup alongside the completion item

---

## Task List

See `docs/roadmap/tasks/v0-15-0.md` for details.

| Task ID | Title | Issue |
|---------|-------|-------|
| — | Tracking: SQL completion feature roadmap | #155 |
| — | Multi-table JOIN column completion | #145 |
| — | INSERT and UPDATE context support | #146 |
| — | Subquery context analysis | #147 |
| — | Schema-qualified table completion (PostgreSQL) | #148 |
| — | SQL function and aggregate completion | #149 |
| — | Snippet completion for common SQL clauses | #150 |
| — | History-ranked completion candidates | #151 |
| — | Fuzzy matching for completion candidates | #152 |
| — | Show column type in completion item detail | #153 |
| — | Keyword documentation popup | #154 |

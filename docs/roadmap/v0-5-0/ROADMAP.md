# v0.5.0 — Result Table Polish

> **Theme**: Browse large datasets comfortably. Differentiate from competitors through result display quality.
> **Prerequisite**: v0.4.0

---

## Goal

Bring the result table to a quality level that surpasses DBeaver and TablePlus.
Display tens of thousands of rows smoothly with virtual scroll, improve NULL visibility,
and complete copy / sort / preview functionality for a "readable and usable" experience.

---

## Exit Criteria

- [ ] 100,000-row results render smoothly via virtual scroll
- [ ] NULL cells display a badge ("NULL" in a muted color)
- [ ] Clicking a column header sorts ascending/descending (fetched data only)
- [ ] Selecting a cell shows its full content in a bottom preview pane (supports long text and JSON)
- [ ] Ctrl+C copies the cell value to the clipboard
- [ ] Right-click context menu offers "Copy cell value / Copy row / Copy as TSV"
- [ ] Page size can be set to 100 / 500 / 1000 rows (saved to config)

---

## Scope

| Category | Content |
|----------|---------|
| Virtual scroll | Slint ListView viewport-only rendering |
| NULL badge | Visualize None-valued cells (muted NULL label) |
| Column sort | Client-side ascending/descending sort |
| Bottom preview pane | Always-visible area showing full content of the selected cell |
| Copy | Ctrl+C for cell value, right-click context menu |
| Pagination | 100/500/1000 row selector + config.toml persistence |

---

## Out of Scope

- Cell editing and UPDATE statements (intentionally excluded from MVP)
- Export (v0.7.0)
- Server-side sort and pagination (future)

---

## Key Risks

- **Slint virtual scroll**: `ListView` renders only the viewport, but variable-height rows can cause issues — fix row height in the design
- Virtual scroll with `VecModel` holds all rows in memory; use pagination to limit memory when needed
- Client-side sort applies only to fetched rows, not to pages that have been truncated — make this clear in the UI
- Right-click context menu requires custom work in Slint (PopupWindow or custom implementation)

---

## Task List

See `docs/roadmap/tasks/v0-5-0.md` for details.

| Task ID | Title | Issue |
|---------|-------|-------|
| T051 | Virtual scroll implementation for result table (Slint ListView) | #34 |
| T052 | NULL badge display in result table | #35 |
| T053 | Client-side column sort in result table | #36 |
| T054 | Bottom preview pane for full cell content | #37 |
| T055 | Copy from result table (Ctrl+C + right-click context menu) | #38 |
| T056 | Pagination row-count selector (100 / 500 / 1000) | #39 |

# Wellfeather — Keyboard Shortcuts Reference

---

## Global — Pane Focus Navigation

These shortcuts work from any pane (no modal must be open).

| Key | Action | Condition |
|-----|--------|-----------|
| `Alt+→` | Move focus: Sidebar → Editor | Focus is on Sidebar |
| `Alt+←` | Move focus: Editor → Sidebar | Focus is on Editor |
| `Alt+↓` | Move focus: Editor → Result panel | Focus is on Editor; result panel is open |
| `Alt+↑` | Move focus: Result panel → Editor | Focus is on Result panel |

> Pane borders are highlighted in blue (`#89b4fa`) to show which pane is active.

---

## Editor (SQL Editor)

### Custom shortcuts

| Key | Action |
|-----|--------|
| `Ctrl+Enter` | Execute SQL statement at the cursor |
| `Ctrl+Shift+Enter` | Execute entire editor content |
| `Shift+Enter` | Execute selected text only |
| `Esc` | Cancel running query |
| `Ctrl+Shift+F` | Format SQL |
| `Ctrl+Space` | Manually show completion candidates |
| `Ctrl+J` | Toggle result panel open / closed |
| `↑` / `↓` | Move cursor up / down one line (timer-based, no key-repeat lag) |
| `Shift+↑` / `Shift+↓` | Extend selection up / down one line |

### Standard text-editing shortcuts (via OS / TextInput)

| Key | Action |
|-----|--------|
| `Ctrl+C` | Copy selection |
| `Ctrl+X` | Cut selection |
| `Ctrl+V` | Paste |
| `Ctrl+A` | Select all |
| `Ctrl+Z` | Undo |
| `Ctrl+Y` | Redo |
| `Shift+Arrow` | Extend selection |
| `Ctrl+←` / `Ctrl+→` | Move by word |
| `Home` / `End` | Move to line start / end |
| `Ctrl+Home` / `Ctrl+End` | Move to document start / end |

---

## Sidebar

The sidebar must be focused (`Alt+←` or click) for these keys to work.

| Key | Action |
|-----|--------|
| `↑` / `↓` | Move keyboard focus up / down in the tree |
| `→` | Expand node; or move to first child if already expanded |
| `←` | Collapse node; or jump to parent node if already collapsed |
| `Enter` | Open table / view (leaf nodes); toggle expand (connection / category nodes) |

> The focused row is highlighted in blue. The cursor is clamped when the tree model changes (e.g. after expand/collapse).

---

## Completion Popup

The completion popup appears automatically while typing in the SQL editor.

| Key | Action |
|-----|--------|
| `↑` / `↓` | Navigate candidates up / down |
| `Enter` | Accept selected candidate |
| `Tab` | Cycle to next candidate |
| `Esc` | Close popup without accepting |
| `;` / ` ` / `,` / `)` / `(` | Accept and auto-close popup |

---

## Result Table

The result table must be focused (`Alt+↓` or click) for these keys to work.

### Row mode (default)

| Key | Action |
|-----|--------|
| `↑` / `↓` | Move selected row |
| `Page Up` / `Page Down` | Move by one viewport page |
| `Home` / `End` | First / last row |
| `Enter` | Enter cell mode (column 0 of current row) |
| `f` | Enter search mode |
| `Esc` | Deselect / clear active filter |
| `Ctrl+C` | Copy selected cell value |

### Cell mode (entered with `Enter` from row mode)

Navigates within a single row. `Esc` returns to row mode.

| Key | Action |
|-----|--------|
| `←` / `→` | Move to previous / next column |
| `Home` / `End` | First / last column |
| `Esc` | Return to row mode |
| `Ctrl+C` | Copy current cell value |

### Search mode (entered with `f` from row mode)

A search bar appears at the bottom of the panel. All keystrokes are captured by the search input.

| Key | Action |
|-----|--------|
| `Enter` | Apply filter → return to row mode |
| `Esc` | Cancel → return to row mode (no filter change) |

#### Search query format

| Input example | Behaviour |
|---------------|-----------|
| `山田太郎` | Substring match across **all** columns |
| `name = '山田太郎'` | Exact match on the **`name`** column |

Filtering is client-side (current result set only). An active-filter indicator is shown above the search bar while a filter is applied.

---

## Connection Form (Modal)

Standard form navigation — no custom shortcuts.

| Key | Action |
|-----|--------|
| `Tab` / `Shift+Tab` | Move between fields |
| `Enter` | Submit form |
| `Esc` | Close modal (via Cancel button equivalent) |

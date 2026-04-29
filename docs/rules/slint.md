# Wellfeather — Slint Coding Rules

For Rust↔Slint integration patterns (weak refs, VecModel, invoke_from_event_loop, etc.)
see `docs/specs/reference-patterns.md`.

---

## 1. Property Direction — Strict Usage

Every property declaration must use the correct direction qualifier.
Never default to `in-out` just because it is easier.

| Qualifier | Meaning | When to use |
|-----------|---------|-------------|
| `in` | Parent/Rust sets the value; this component only reads | Data flowing in from outside |
| `out` | This component computes or owns the value; parent reads | State this component controls |
| `in-out` | Both parent and child can write | Genuinely bidirectional (e.g., a controlled input) |
| *(none)* | Private, internal only | Not accessible outside the component |

```slint
// ✅ Correct direction usage
component ResultTable {
    in property <[RowData]> rows;           // parent sets row data
    in property <bool> is-loading;          // parent tells us we're loading
    out property <int> selected-row: -1;    // we track which row is selected
    in-out property <string> search-text;   // parent can pre-fill; user can also edit
}

// ❌ Wrong — everything in-out is lazy and breaks data-flow reasoning
component ResultTable {
    in-out property <[RowData]> rows;
    in-out property <bool> is-loading;
    in-out property <int> selected-row;
}
```

---

## 2. One File, One Public Component

Each `.slint` file exports exactly one public component as its primary entry point.
Private sub-components used only within the same file are allowed.

```
app/src/ui/components/
├── result_table.slint          ← exports ResultTable
├── result_table_header.slint   ← exports ResultTableHeader
├── result_table_body.slint     ← exports ResultTableBody
├── editor.slint                ← exports Editor
└── sidebar.slint               ← exports Sidebar
```

```slint
// result_table.slint
// Private helper — only used in this file
component RowHighlight { ... }

// Public export — one per file
export component ResultTable { ... }
```

Do not accumulate multiple unrelated components in one file to save on file count.

---

## 3. No Component-Local Application State

`global UiState` is the single source of truth for all application state.
Components must not cache or duplicate application state locally.

```slint
// ❌ Component caches connection status locally
component Sidebar {
    property <bool> is-connected: false;  // duplicates UiState.is-connected

    init => {
        // now you have two sources of truth
        is-connected = UiState.is-connected;
    }
}

// ✅ Read directly from the global
component Sidebar {
    // No local copy; binding reads from UiState on every access
    Text { text: UiState.active-connection-name; }
}
```

**Exceptions** — local state is allowed for purely visual, transient concerns:

- Hover / press / focus visual state (`property <bool> is-hovered`)
- Animation intermediate values
- Popup open/closed state that has no meaning outside this component

When popup state matters to Rust (e.g., "is the connection dialog open"), it must live in `UiState`.

---

## 4. Logic Minimization in `.slint`

`.slint` files are for layout, styling, and simple data binding.
Business logic lives in Rust and is passed in through properties.

```slint
// ✅ Simple conditional for visual toggling
Rectangle {
    background: is-selected ? Theme.selection-color : Theme.row-color;
}

// ✅ Simple ternary for text
Text { text: row-count == 0 ? @tr("0 rows") : @tr("{0} rows", row-count); }

// ❌ Business logic in .slint
property <bool> can-run: sql-text.to-lowercase().starts-with("select")
                       && !is-loading
                       && active-connection != "";
// → compute `can-run` in Rust, expose as `in property <bool> can-run`
```

**Allowed in `.slint`:**
- Ternary expressions for style/text switching
- Simple arithmetic for layout (e.g., `width / 2`)
- `@tr()` string interpolation with positional args

**Not allowed in `.slint`:**
- String parsing or manipulation beyond `@tr()`
- Multi-step conditional chains
- Loops with logic
- Anything involving data transformation

---

## 5. Localization — `@tr()` Mandatory

Every user-visible string in a `.slint` file must be wrapped in `@tr()`.
Hardcoded English strings are prohibited.

```slint
// ❌ Hardcoded string
Text { text: "Add Connection"; }
Text { text: "Running..."; }

// ✅ Localized
Text { text: @tr("Add Connection"); }
Text { text: @tr("Running\u{2026}"); }

// ✅ With interpolation (positional args)
Text { text: @tr("{0} rows", row-count); }
Text { text: @tr("{0} / {1} rows", filtered-count, total-count); }
```

See `app/lang/en/LC_MESSAGES/wellfeather.po` and `app/lang/ja/LC_MESSAGES/wellfeather.po`
for the translation files. Add new strings to both `.po` files when adding UI text.

---

## 6. Naming Conventions

| Element | Convention | Example |
|---------|-----------|---------|
| Component names | `PascalCase` | `ResultTable`, `ConnectionForm` |
| Properties | `kebab-case` | `result-rows`, `is-loading` |
| Callbacks | `kebab-case` | `run-query`, `on-row-selected` |
| Animations / transitions | `kebab-case` | `fade-duration` |
| Global components | `PascalCase` | `UiState`, `Theme`, `Typography` |

Callback names express the **event that occurred**, not the handler name:

```slint
// ✅ Event-named callback
callback row-selected(int);      // "a row was selected"
callback run-query(string);      // "run query was requested"

// ❌ Handler-named
callback handle-row-click(int);
callback on-run-button-pressed(string);
```

---

## 7. Theme and Typography Tokens

Never use literal color values or font sizes in component code.
Always reference `Theme.*` and `Typography.*` globals.

```slint
// ❌ Literal values
Text {
    color: #cdd6f4;
    font-size: 12px;
}
Rectangle { background: #1e1e2e; }

// ✅ Token references
Text {
    color: Theme.text-primary;
    font-size: Typography.size-base;
}
Rectangle { background: Theme.surface; }
```

Typography scale (defined in `common.slint`):

| Token | Size |
|-------|------|
| `Typography.size-xs` | 9px |
| `Typography.size-sm` | 10px |
| `Typography.size-md` | 11px |
| `Typography.size-base` | 12px |
| `Typography.size-lg` | 13px |
| `Typography.size-xl` | 15px |
| `Typography.size-2xl` | 16px |

---

## 8. Timer Usage in `.slint`

Slint-side `Timer` elements are for **purely visual** purposes only
(animations, auto-hide toasts, blinking cursors).

Do **not** use `.slint` timers to drive application logic or data polling.
Application-level timing (debounce, key-repeat, completion delay) uses `slint::Timer` in Rust.

```slint
// ✅ Visual-only timer — auto-dismiss a status indicator
Timer {
    interval: 3000ms;
    running: status-visible;
    triggered => { status-visible = false; }
}

// ❌ Logic timer — polling or data-driving
Timer {
    interval: 100ms;
    running: true;
    triggered => {
        // updating result data — this belongs in Rust
        result-rows = fetch-new-rows();
    }
}
```

---

## 9. Key Handling Patterns

Custom key handling (overriding OS defaults) uses `FocusScope` with `capture-key-pressed`.

- Use `capture-key-pressed` (capture phase) to intercept keys before `TextInput` receives them.
- Return `EventResult.accept` to consume the key; `EventResult.reject` to pass it through.
- For held-key navigation, implement the two-phase heartbeat pattern documented in `editor.slint`.

```slint
// ✅ Intercept UP/DOWN to prevent TextInput's built-in OS repeat lag
FocusScope {
    capture-key-pressed(event) => {
        if event.text == Key.DownArrow {
            root.down-key-seen = true;
            if !root.down-held {
                root.down-held = true;
                // move cursor once immediately
            } else {
                root.down-repeat-received = true;
            }
            return EventResult.accept;
        }
        EventResult.reject
    }
}
```

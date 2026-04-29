# Wellfeather — Code Guidelines

General conventions that apply across all Rust code in the workspace.
For Rust-specific patterns see `docs/rules/rust.md`.
For Slint-specific rules see `docs/rules/slint.md`.

---

## Naming Conventions

### Rust

| Element | Convention | Example |
|---------|-----------|---------|
| Types, traits, enums | `PascalCase` | `DbService`, `CompletionEngine`, `QueryResult` |
| Functions, methods, variables | `snake_case` | `execute_query`, `conn_id`, `row_count` |
| Constants, statics | `SCREAMING_SNAKE_CASE` | `MAX_HISTORY_ENTRIES`, `DEFAULT_PORT` |
| Modules, files | `snake_case` | `connection_state.rs`, `mod service` |
| Lifetime parameters | short, lowercase | `'a`, `'conn` |
| Type parameters | `PascalCase`, single letter for generics | `T`, `E`, `ConnId` |

### Slint

| Element | Convention | Example |
|---------|-----------|---------|
| Component names | `PascalCase` | `ResultTable`, `ConnectionForm` |
| Properties, callbacks | `kebab-case` | `result-rows`, `run-query` |
| Global components | `PascalCase` | `UiState`, `Theme`, `Typography` |

---

## Comment Policy

Write **no comments** by default. Code with clear names is self-documenting.

Add a comment **only when the WHY is non-obvious**:
- A hidden constraint or external requirement that isn't visible in the code
- A subtle invariant that must hold but cannot be expressed in types
- A deliberate workaround for a specific bug or limitation
- Behavior that would surprise a careful reader

```rust
// ❌ Explains WHAT — obvious from the code
// Increment the counter
counter += 1;

// ❌ References the task or caller — rots over time
// Added for the query cancellation flow (issue #73)
token.cancel();

// ✅ Explains WHY — non-obvious constraint
// invoke_from_event_loop only errors if the event loop has already stopped;
// at this point in the shutdown sequence that is expected, so we ignore it.
let _ = slint::invoke_from_event_loop(move || { ... });

// ✅ Documents the poison-recovery pattern (non-obvious Rust idiom)
// unwrap_or_else recovers from a poisoned lock without propagating the panic.
let data = self.inner.read().unwrap_or_else(|p| p.into_inner());
```

**One short line max.** No multi-paragraph docstrings. No multi-line comment blocks.

### Clone annotation (mandatory)

Every `.clone()` call that exists to satisfy a closure's ownership requirement must have a comment:

```rust
// clone required: tokio::spawn requires 'static
let ctx = ctx.clone();
// clone required: each callback closure needs owned weak ref
let window_weak = window_weak.clone();
```

---

## Import Organization

Group imports in this order, separated by blank lines:

1. `std` / `core` / `alloc`
2. External crates (`tokio`, `sqlx`, `slint`, `anyhow`, etc.)
3. Workspace crates (`wf_db`, `wf_config`, etc.)
4. Local module (`super::`, `crate::`)

```rust
use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Context;
use slint::VecModel;
use tokio::sync::mpsc;

use wf_db::{DbService, QueryResult};
use wf_config::ConfigManager;

use crate::app::command::Command;
use crate::state::AppState;
```

Do not use `use super::*;` outside `#[cfg(test)]` blocks.

---

## Module Organization

- Keep `mod.rs` as a thin re-export hub. Move logic into named submodules.
- Only export what is part of the public API. Internal types and helpers stay private.
- Avoid deeply nested module hierarchies — two levels is usually enough.

```rust
// ✅ Thin mod.rs
// src/app/mod.rs
pub mod command;
pub mod controller;
pub mod event;
pub mod session;
pub mod localized_message;

pub use localized_message::LocalizedMessage;
```

---

## No Temporary / Debug Code in Commits

Do not commit:
- `todo!()` / `unimplemented!()` / `unreachable!()` without a comment and tracking issue
- `println!` / `dbg!` / `eprintln!` debug statements
- `#[allow(...)]` without a comment explaining why the lint is suppressed
- Commented-out code blocks

```rust
// ❌ Do not commit
todo!("implement cancellation");
println!("DEBUG: result = {:?}", result);
// let old_impl = ...

// ✅ If something is genuinely not yet implemented, use a typed placeholder
Err(anyhow::anyhow!("not yet implemented: cancellation (see #73)"))
```

---

## No-Op Backwards-Compatibility Shims

Do not add:
- Unused `_variable` prefixes for removed parameters
- Re-exports of deleted types with a `// removed` comment
- Empty `impl` blocks retained for "future use"

If something is unused, delete it. Git history records what was removed and why.

---

## File Length

There is no hard line limit, but files over ~500 lines are a signal to consider splitting.
Split along logical boundaries (e.g., separate the service from its models).

# Wellfeather — Rust Coding Standards

## References

- [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
- [Async Book](https://rust-lang.github.io/async-book/)
- [Tokio Tutorial](https://tokio.rs/tokio/tutorial)
- [Rust Performance Book](https://nnethercote.github.io/perf-book/)

---

## Error Handling

### Crate-level policy

| Layer | Tool | Usage |
|-------|------|-------|
| Library crates (`wf-db`, `wf-config`, `wf-query`, `wf-completion`, `wf-history`) | `thiserror` | Typed, match-able error enums |
| `app/` binary, `AppController` | `anyhow` | Contextual error propagation |

```rust
// ✅ wf-db/src/error.rs — typed error for library crate
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),
    #[error("Query execution error: {0}")]
    QueryError(String),
    #[error("Query cancelled")]
    Cancelled,
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
}

// ✅ app/src/app/controller.rs — anyhow for application layer
pub async fn handle_run_query(&self, sql: &str) -> anyhow::Result<()> {
    let result = self.db.execute(conn_id, sql)
        .await
        .context("Query execution failed")?;
    Ok(())
}
```

### `unwrap()` / `expect()` policy

`unwrap()` and `expect()` are **only permitted inside `#[cfg(test)]` blocks.**

```rust
// ❌ FORBIDDEN in production code
let pool = self.pools.get(id).unwrap();
let lock = state.read().unwrap();

// ✅ Use ? propagation
let pool = self.pools.get(id)
    .ok_or_else(|| DbError::ConnectionFailed(id.to_string()))?;

// ✅ RwLock poison recovery — the ONE exception pattern (not unwrap)
let data = self.inner.read().unwrap_or_else(|p| p.into_inner());

// ✅ In tests, unwrap is acceptable
#[test]
fn connect_should_add_pool() {
    let result = service.connect(&conn).await.unwrap();
}
```

> `unwrap_or_else(|p| p.into_inner())` for RwLock poison is an established project pattern,
> not an exception to the `unwrap` ban — it explicitly handles the poison case.

### User-facing errors

All error types displayed in the UI must implement `LocalizedMessage` and return translated strings via `t!()`.

```rust
// app/src/app/localized_message.rs
impl LocalizedMessage for DbError {
    fn localized_message(&self) -> String {
        match self {
            DbError::ConnectionFailed(s) => t!("error.db_connect_failed", reason = s).to_string(),
            DbError::Cancelled           => t!("error.query_cancelled").to_string(),
            // ...
        }
    }
}
```

---

## Async Patterns

### Tokio task spawning

All DB operations and long-running work must run inside `tokio::spawn`.
Never block the event loop thread (the thread running `slint::run_event_loop`).

```rust
// ✅ Correct: DB work in a spawned task, UI update via invoke_from_event_loop
ui.on_run_query(move |sql| {
    let sql = sql.to_string();
    // clone required: tokio::spawn requires 'static
    let window_weak = window_weak.clone();
    let ctx = ctx.clone();

    tokio::spawn(async move {
        let result = ctx.db.execute(&sql).await;
        slint::invoke_from_event_loop(move || {
            if let Some(window) = window_weak.upgrade() {
                // update UI
            }
        }).unwrap(); // invoke_from_event_loop only fails if event loop is stopped
    });
});
```

### Never hold a lock across `.await`

```rust
// ❌ FORBIDDEN: holding RwLock guard across an await point
async fn bad(&self) {
    let guard = self.state.read().unwrap_or_else(|p| p.into_inner());
    some_async_fn().await; // guard is still held here — potential deadlock
}

// ✅ Extract the value before awaiting
async fn good(&self) {
    let conn_id = {
        let data = self.state.read().unwrap_or_else(|p| p.into_inner());
        data.active_id.clone()
    }; // guard dropped here
    some_async_fn(&conn_id).await;
}
```

### Channel communication

Use `tokio::sync::mpsc` channels to communicate between UI and backend.
Never call service methods directly from Slint callbacks.

```rust
// ✅ UI → Controller via Command channel
tx_cmd.send(Command::RunQuery(sql)).await?;

// ✅ Controller → UI via Event channel
tx_event.send(Event::QueryFinished(result)).await?;
```

---

## Concurrency and Shared State

### `Arc<AppState>` pattern

`AppState` is shared via `Arc`. Each sub-state uses an inner `RwLock`.
Access state only through methods — never reach into the `RwLock` directly.

```rust
// ❌ Do not access RwLock from outside the state module
let data = state.query.data.read()...; // FORBIDDEN

// ✅ Use the provided methods
state.query.set_loading(true);
let result = state.query.result();
```

### UI-thread-only state

Mutable state shared only across UI-thread closures uses `Rc<RefCell<T>>`, not `Arc<Mutex<T>>`.
Slint's event loop is single-threaded.

```rust
// ✅ Debounce timer shared across closures — UI thread only
let debounce: Rc<RefCell<Option<slint::Timer>>> = Rc::new(RefCell::new(None));
let debounce_clone = debounce.clone(); // clone required: shared across closures

ui.on_text_changed(move |_| {
    *debounce_clone.borrow_mut() = None; // cancels previous timer
    // ...
});
```

### Clone annotation

Every `.clone()` call in a closure must have a preceding comment explaining why.

```rust
// clone required: tokio::spawn requires 'static + Send
let ctx = ctx.clone();
// clone required: each callback closure needs owned weak ref
let window_weak = window_weak.clone();
```

---

## Logging

Use `tracing` throughout. Never use `println!` or `eprintln!` in production code.

```rust
// ✅ Structured logging with field names
tracing::info!(conn_id = %id, "Connected to database");
tracing::debug!(sql = %sql, duration_ms = %ms, "Query executed");
tracing::warn!("Metadata cache flush failed: {}", e);
tracing::error!(error = ?e, "Unexpected error in controller");
```

Log levels:
- `error`: unexpected failures that affect correctness
- `warn`: recoverable issues, degraded operation
- `info`: lifecycle events (connect, disconnect, query start/finish)
- `debug`: detailed execution trace (SQL text, timing)

---

## Type Design

### Prefer named structs over tuples

```rust
// ❌ Opaque tuple
fn extract(&self) -> (String, usize) { ... }

// ✅ Named return
pub struct ExtractionResult { pub sql: String, pub cursor: usize }
fn extract(&self) -> ExtractionResult { ... }
```

### Newtype for domain IDs

```rust
// ✅ Connection IDs are strings, but wrapping prevents accidental misuse
pub struct ConnectionId(pub String);
```

### Builder pattern for complex construction

Use builder pattern when a struct has more than 3 optional fields. Keep mandatory fields in `new()`.

---

## Code Quality

### Iterators over manual loops

```rust
// ❌ Manual accumulation
let mut titles = Vec::new();
for row in &rows {
    titles.push(row[0].clone().unwrap_or_default());
}

// ✅ Iterator pipeline
let titles: Vec<_> = rows.iter()
    .map(|row| row[0].clone().unwrap_or_default())
    .collect();
```

### No `unsafe` without justification

`unsafe` blocks require a `// SAFETY:` comment explaining why the invariants hold.

```rust
// ✅ Documented unsafe
// SAFETY: `ptr` is valid for the lifetime of this function and aligned to T.
let value = unsafe { ptr.read() };
```

### No dead code in committed branches

Remove unused `use` statements, functions, and variables before committing.
`#[allow(dead_code)]` requires a comment explaining why the code is retained.

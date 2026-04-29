# Wellfeather — Performance Rules

## References

- [Rust Performance Book](https://nnethercote.github.io/perf-book/)
- [Tokio Performance Guide](https://tokio.rs/tokio/topics/performance)
- [Slint Language Reference](https://slint.dev/releases/latest/docs/slint/)
- [Criterion Benchmarking](https://bheisler.github.io/criterion.rs/book/)

---

## Performance Targets

These targets apply to **release builds** (`just build-release`).

| Metric | Target | Measurement |
|--------|--------|-------------|
| Startup time (cold) | **< 1 second** | `time ./wellfeather` |
| Startup time (warm) | **< 0.5 seconds** | same, after config loaded |
| Idle memory | **< 50 MB** | OS process monitor |
| Memory with 100k rows | **< 200 MB** | OS process monitor |
| Query result UI update | **< 200 ms** (≤ 10k rows) | measured from query finish to first paint |
| Completion popup latency | **< 100 ms** | measured from keystroke to popup visible |

These are verified manually before each release. See v1.0.0 tasks for the measurement checklist.

---

## Slint Rendering Rules

Slint performs a **full-scene GPU re-render** on every dirty mark. Each property change can
trigger a re-render of all visible elements. These rules prevent unnecessary re-renders.

### 1. Never update VecModel element-by-element

```rust
// ❌ Each set_row_data() triggers a re-render
for (i, row) in new_rows.iter().enumerate() {
    model.set_row_data(i, row.clone());
}

// ✅ Rebuild the model atomically — one dirty mark
let model = Rc::new(slint::VecModel::from(rows_to_ui(&result)));
ui.set_result_rows(model.into());
```

### 2. Batch all UI updates in a single `invoke_from_event_loop` closure

```rust
// ❌ Two separate closures = two re-renders
slint::invoke_from_event_loop({
    let w = window_weak.clone();
    move || { w.upgrade().unwrap().global::<UiState>().set_is_loading(false); }
}).unwrap();
slint::invoke_from_event_loop({
    let w = window_weak.clone();
    move || { w.upgrade().unwrap().global::<UiState>().set_result_rows(model.into()); }
}).unwrap();

// ✅ Single closure = single re-render
let model = Rc::new(slint::VecModel::from(rows)); // build outside closure
slint::invoke_from_event_loop(move || {
    if let Some(window) = window_weak.upgrade() {
        let ui = window.global::<UiState>();
        ui.set_is_loading(false);
        ui.set_result_rows(model.into()); // Rc cannot cross thread boundary; build before moving
    }
}).unwrap();
```

> **Note:** `Rc` cannot be sent across threads. Convert `Vec<RowData>` in the tokio task, then create `VecModel` inside `invoke_from_event_loop`.

### 3. Keep dirty regions minimal during cursor movement

Cursor movement in the SQL editor triggers a re-render. To prevent the result table
(potentially 100+ visible cells) from re-rendering on every keypress:

- Cap the editor/gutter height to `parent.height - panel-height` when the result panel is open.
  This keeps the editor's screen region entirely above the panel, so Slint's dirty-region
  detection does not overlap result table cells.
- Do not change result table properties in response to cursor movement.

### 4. Virtual scroll — render only the visible row range

The result table renders only the rows currently visible in the viewport.
The VecModel always contains **all** rows; row count is not capped.
Only row `height * viewport_offset / row_height` through `visible_height / row_height` rows
are instantiated as Slint elements.

### 5. Avoid deep property binding chains

Long binding chains (`a → b → c → d`) cause the entire chain to re-evaluate on any change.
If a derived value is expensive, compute it in Rust and pass it as an `in` property instead.

```slint
// ❌ Long chain — recomputes on every tiny change
property <int> visible-count: total-rows - filtered-rows - hidden-rows - ...;

// ✅ Compute in Rust, expose as a single property
in property <int> visible-count;
```

---

## Async and Threading Rules

### Never block the event loop thread

The Slint event loop thread must never block. Blocking it causes UI freezes.

```rust
// ❌ Blocking call on event loop thread
ui.on_run_query(move |sql| {
    let result = runtime.block_on(db.execute(&sql)); // FORBIDDEN
});

// ✅ Spawn a tokio task; return to UI via invoke_from_event_loop
ui.on_run_query(move |sql| {
    let sql = sql.to_string();
    // clone required: tokio::spawn requires 'static
    let ctx = ctx.clone();
    let window_weak = window_weak.clone();
    tokio::spawn(async move {
        let result = ctx.db.execute(&sql).await;
        slint::invoke_from_event_loop(move || { /* update UI */ }).unwrap();
    });
});
```

### Use `CancellationToken` for query cancellation

```rust
let token = CancellationToken::new();
state.query.set_cancel_token(token.clone());

tokio::select! {
    result = db.execute_with_cancel(conn_id, sql, token.clone()) => { /* finished */ }
    _ = token.cancelled() => { /* cancelled by user */ }
}
```

### Avoid `Mutex` for hot paths

Prefer channels (`mpsc`, `oneshot`) over `Mutex` for communication between async tasks.
`Mutex` is acceptable for low-frequency operations (config save, metadata cache writes).

---

## Memory Management

### Avoid unnecessary clones in hot paths

```rust
// ❌ Cloning a large result just to pass it around
let rows = result.rows.clone();
process(&rows);

// ✅ Borrow or pass by reference
process(&result.rows);
```

### Pre-allocate collections when size is known

```rust
// ✅ Avoid repeated reallocations
let mut cells = Vec::with_capacity(columns.len());
for col in &columns {
    cells.push(/* ... */);
}
```

### Clear buffers instead of dropping and reallocating

```rust
// ✅ Reuse allocation for repeated operations (e.g., row rendering buffer)
self.row_buffer.clear(); // clears contents but keeps capacity
```

---

## Benchmarking with Criterion

Use [Criterion](https://bheisler.github.io/criterion.rs/book/) to benchmark critical paths.
Benchmarks live in `benches/` within the relevant crate.

### Where to add benchmarks

| Crate | Target | Benchmark file |
|-------|--------|---------------|
| `wf-completion` | `CompletionEngine::complete()` | `crates/wf-completion/benches/engine.rs` |
| `wf-query` | `extract_statement_at()`, CSV/JSON export | `crates/wf-query/benches/analyzer.rs` |
| `app` | `rows_to_ui()` VecModel conversion | `app/benches/ui_conversion.rs` |

### Benchmark structure

```rust
// crates/wf-completion/benches/engine.rs
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};

fn bench_complete(c: &mut Criterion) {
    let mut group = c.benchmark_group("completion_engine");

    for prefix in ["SEL", "SELECT * FR", "SELECT * FROM users WHERE "].iter() {
        group.bench_with_input(
            BenchmarkId::new("complete", prefix.len()),
            prefix,
            |b, prefix| {
                let engine = build_test_engine();
                b.iter(|| engine.complete(black_box(prefix), black_box(prefix.len())));
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_complete);
criterion_main!(benches);
```

### Running benchmarks

```bash
cargo bench -p wf-completion
cargo bench -p wf-query
```

Benchmarks are **not** run in CI by default. Run them manually before performance-sensitive
changes and compare with `--save-baseline` / `--load-baseline`.

```bash
# Save a baseline before your change
cargo bench -p wf-completion -- --save-baseline before

# Run again after your change and compare
cargo bench -p wf-completion -- --load-baseline before --save-baseline after
```

---

## Profiling

For startup time measurement:

```bash
# Windows (PowerShell)
Measure-Command { .\target\release\wellfeather.exe }

# macOS/Linux
time ./target/release/wellfeather
```

For memory profiling on Windows, use Task Manager or Process Hacker to observe the working set.
On macOS use Instruments; on Linux use `heaptrack` or `/proc/<pid>/status`.

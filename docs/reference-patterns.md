# Slint + Rust Integration Patterns

> Reference project: `docs/master-password/`
> Key reference files:
> - `app/src/ui/mod.rs` — central Slint↔Rust integration
> - `app/src/app_context.rs` — state management implementation
> - `app/build.rs` — build configuration

---

## 1. Unified UI State Management with Slint `global`

Consolidate all properties and callbacks into a single `global` component.
Access it from Rust via `window.global::<UiState>()`.

```slint
// app.slint
export global UiState {
    // Properties (in-out = bidirectional between Rust and Slint)
    in-out property <bool>   is_loading: false;
    in-out property <string> current_screen: "main";
    in-out property <string> status_message: "";
    in-out property <[RowData]> result_rows: [];

    // Callbacks (Slint → Rust)
    callback run_query(string);
    callback cancel_query();
    callback connect(string);
    callback disconnect(string);
}
```

```rust
// Rust side: reading and writing properties
let ui = window.global::<UiState>();
ui.set_is_loading(true);
ui.set_status_message("Running...".into());
let rows = ui.get_result_rows();  // read
```

**Reference**: `app/src/ui/app.slint` L43-252, `app/src/ui/mod.rs` L169-207

---

## 2. Split Callback Registration Pattern

Split into `register_*_callbacks()` functions and batch-register them in `UI::new()`.
This prevents logic from accumulating in `main.rs` or `new()`.

```rust
// ui/mod.rs
pub struct UI {
    window: AppWindow,
}

impl UI {
    pub fn new(ctx: AppContext) -> Result<Self, AppError> {
        let window = AppWindow::new()?;

        Self::init_ui_state(&window, &ctx);
        // Register callbacks split by concern
        Self::register_query_callbacks(&window, ctx.clone());
        Self::register_connection_callbacks(&window, ctx.clone());
        Self::register_settings_callbacks(&window, ctx.clone());
        Self::register_export_callbacks(&window, ctx.clone());

        Ok(Self { window })
    }

    pub fn run(&self) -> Result<(), AppError> {
        self.window.run().map_err(|e| AppError::Ui(e.to_string()))
    }

    fn register_query_callbacks(window: &AppWindow, ctx: AppContext) {
        let ui = window.global::<UiState>();
        // ... register callbacks
    }
}
```

**Reference**: `app/src/ui/mod.rs` L89-166

---

## 3. Window References Inside Closures: `as_weak()` Pattern

Capturing the window directly in a callback closure creates a cycle.
Always take a weak reference with `as_weak()` and call `upgrade()` when needed.

```rust
fn register_query_callbacks(window: &AppWindow, ctx: AppContext) {
    let ui = window.global::<UiState>();

    // WRONG: do not capture window directly in a closure
    // ui.on_run_query(move |sql| { window.global::<UiState>()... });

    // OK: use a weak reference
    let window_weak = window.as_weak();
    ui.on_run_query(move |sql| {
        if let Some(window) = window_weak.upgrade() {
            window.global::<UiState>().set_is_loading(true);
        }
    });
}
```

**Reference**: `app/src/ui/mod.rs` L641-711

---

## 4. `ctx.clone()` Pattern Inside Callbacks

Each callback requires ownership of `AppContext`, so clone it before passing it to the closure.
`AppContext` is a wrapper around `Arc<RwLock<Inner>>`, so cloning is just a pointer copy.

```rust
fn register_query_callbacks(window: &AppWindow, ctx: AppContext) {
    let ui = window.global::<UiState>();

    // Clone per callback
    {
        // clone required: callback closure needs owned ctx
        let ctx = ctx.clone();
        let window_weak = window.as_weak();
        ui.on_run_query(move |sql| {
            // use ctx to handle the operation
        });
    }
    {
        // clone required: callback closure needs owned ctx
        let ctx = ctx.clone();
        ui.on_cancel_query(move || {
            ctx.cancel_query();
        });
    }
}
```

> **Convention**: Always annotate clones with `// clone required: <reason>`.
> (See `app/src/ui/mod.rs` L641, L716, etc.)

---

## 5. `Arc<RwLock<Inner>>` + Poison Recovery Pattern

`AppContext`, the core of state management, is shared thread-safely via `Arc<RwLock<Inner>>`.
Always implement `read()` / `write()` helpers that recover from lock poisoning after a panic.

```rust
// app_context.rs
#[derive(Clone)]
pub struct AppContext {
    inner: Arc<RwLock<AppContextInner>>,
}

impl AppContext {
    /// Acquire a read lock, recovering from poisoning
    fn read(&self) -> RwLockReadGuard<'_, AppContextInner> {
        self.inner
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Acquire a write lock, recovering from poisoning
    fn write(&self) -> RwLockWriteGuard<'_, AppContextInner> {
        self.inner
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    // External access only through methods (never expose RwLock directly)
    pub fn is_loading(&self) -> bool {
        self.read().is_loading
    }

    pub fn set_loading(&self, v: bool) {
        self.write().is_loading = v;
    }
}
```

**Reference**: `app/src/app_context.rs` L190-310

---

## 6. Binding Lists with `slint::VecModel`

Use `VecModel` for Slint list properties (`[T]`).
Always implement a paired conversion method: domain model → UI type.

```rust
use std::rc::Rc;

// Convert domain model → Slint UI type
fn rows_to_ui(rows: &QueryResult) -> Vec<crate::RowData> {
    rows.rows.iter().map(|r| crate::RowData {
        // clone required: Slint requires owned SharedString
        cells: r.iter().map(|c| match c {
            Some(v) => v.clone().into(),
            None    => slint::SharedString::default(),
        }).collect::<slint::ModelRc<_>>().into(),
    }).collect()
}

// Reflect in UI
let model = Rc::new(slint::VecModel::from(rows_to_ui(&result)));
ui_state.set_result_rows(model.into());
```

**Reference**: `app/src/ui/mod.rs` L459-482, L452-453

---

## 7. Using `slint::Timer`

Slint's `Timer` runs on the same thread as the event loop.
Use `TimerMode::Repeated` for recurring execution and `TimerMode::SingleShot` for one-shot execution.

```rust
use std::time::Duration;

// Repeating timer (e.g., update elapsed time in the status bar)
let timer = slint::Timer::default();
timer.start(
    slint::TimerMode::Repeated,
    Duration::from_millis(100),
    move || {
        // runs on the UI thread
        if let Some(window) = window_weak.upgrade() {
            // ...
        }
    },
);
// Dropping the timer stops it → keep it in a field to control its lifetime

// One-shot timer (e.g., trigger completion after debounce)
let timer = slint::Timer::default();
timer.start(
    slint::TimerMode::SingleShot,
    Duration::from_millis(300),
    move || {
        // runs once after 300ms
    },
);
```

> **Note**: Dropping a `Timer` stops it immediately.
> Keep it in a field or `Rc<RefCell<Option<Timer>>>` to manage its lifetime.

**Reference**: `app/src/ui/mod.rs` L390-440 (hotkey polling), L1249-1326 (clipboard countdown)

---

## 8. Sharing Mutable State Within the UI Thread with `Rc<RefCell<>>`

When sharing mutable state that lives only on the UI thread across multiple closures,
use `Rc<RefCell<>>` rather than `Arc<Mutex<>>` (Slint is single-threaded).

```rust
// Example: share a debounce timer across multiple closures
let debounce_timer: Rc<RefCell<Option<slint::Timer>>> = Rc::new(RefCell::new(None));

{
    let debounce_timer = debounce_timer.clone();
    let window_weak = window.as_weak();
    ui.on_text_changed(move |text| {
        // Cancel the previous timer (stopped by drop)
        *debounce_timer.borrow_mut() = None;

        let window_weak = window_weak.clone();
        let text = text.to_string();
        let timer = slint::Timer::default();
        timer.start(
            slint::TimerMode::SingleShot,
            Duration::from_millis(300),
            move || {
                if let Some(window) = window_weak.upgrade() {
                    // trigger completion after 300ms
                    trigger_completion(&window, &text);
                }
            },
        );
        *debounce_timer.borrow_mut() = Some(timer);
    });
}
```

**Reference**: `app/src/ui/mod.rs` L1249-1326

---

## 9. Async Processing + UI Updates: `invoke_from_event_loop`

> **This pattern is specific to wellfeather and does not exist in master-password.**
> master-password uses only synchronous operations; wellfeather requires this pattern because DB queries are async.

Slint UI updates can only be performed from the event loop thread.
Use `slint::invoke_from_event_loop` to update the UI from a tokio task.

```rust
fn register_query_callbacks(window: &AppWindow, ctx: Arc<AppState>) {
    let ui = window.global::<UiState>();
    let window_weak = window.as_weak();

    ui.on_run_query(move |sql| {
        let sql = sql.to_string();
        // clone required: tokio::spawn requires 'static
        let window_weak = window_weak.clone();
        let ctx = ctx.clone();

        tokio::spawn(async move {
            // Runs outside the UI thread (tokio runtime)
            let result = ctx.db.execute(&sql).await;

            // Return to the UI thread to update
            slint::invoke_from_event_loop(move || {
                if let Some(window) = window_weak.upgrade() {
                    let ui = window.global::<UiState>();
                    match result {
                        Ok(r)  => {
                            ui.set_is_loading(false);
                            let model = Rc::new(slint::VecModel::from(rows_to_ui(&r)));
                            ui.set_result_rows(model.into());
                        }
                        Err(e) => {
                            ui.set_is_loading(false);
                            ui.set_error_message(e.to_string().into());
                        }
                    }
                }
            }).unwrap();
        });
    });
}
```

> **Note**: `Rc` cannot be used inside the `invoke_from_event_loop` closure.
> Convert to `Vec` before entering the closure, then create the `VecModel` inside it.

---

## 10. Setting Initial UI State (`init_ui_state`)

Before registering callbacks in `UI::new()`, provide an initialization function that reflects the current state into the UI.

```rust
fn init_ui_state(window: &AppWindow, ctx: &AppContext) {
    let ui = window.global::<UiState>();

    // Determine initial screen from state
    ui.set_current_screen(ctx.initial_screen().into());

    // Set each property
    ui.set_is_loading(false);
    ui.set_theme(ctx.theme().into());
    ui.set_font_size(ctx.font_size() as i32);
}
```

**Reference**: `app/src/ui/mod.rs` L169-207

---

## 11. `build.rs` Configuration

Minimal configuration to compile `.slint` files into Rust code.
Register change-detection paths to enable incremental builds.

```rust
// build.rs
fn main() {
    println!("cargo:rerun-if-changed=src/ui/app.slint");

    // Also watch .slint files under components/
    if let Ok(entries) = std::fs::read_dir("src/ui/components") {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "slint") {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
    }

    slint_build::compile("src/ui/app.slint")
        .expect("Failed to compile Slint UI");
}
```

To use components on the Rust side:

```rust
// main.rs or lib.rs
slint::include_modules!();
```

**Reference**: `app/build.rs`

---

## 12. Slint Type Conversions

Rust strings and Slint `SharedString` convert to each other with `.into()`.

```rust
// Rust String / &str → SharedString
ui.set_message("hello".into());
ui.set_message(my_string.clone().into());

// SharedString → Rust String
let s: String = ui.get_message().to_string();

// i32 conversion (Slint represents integers as i32)
ui.set_row_count(result.row_count as i32);
ui.set_font_size(config.font_size as i32);

// bool passes through as-is
ui.set_is_loading(true);
```

---

## 13. Error Handling and UI Display

Display errors inline via a `UiState` error property rather than using dialogs.

```rust
match result {
    Ok(r)  => {
        ui.set_error_message("".into());  // clear error
        // reflect result
    }
    Err(e) => {
        tracing::error!("Query failed: {}", e);
        ui.set_error_message(e.to_string().into());
        ui.set_is_loading(false);
    }
}
```

---

## 14. Common Mistakes and Fixes

| Mistake | Fix |
|---------|-----|
| Capturing the window directly in a closure | Use `as_weak()` + `upgrade()` |
| Updating properties directly from outside the UI thread | Use `invoke_from_event_loop` |
| Using `Rc` inside a tokio task | Convert to `Vec` outside the closure, then pass it in |
| Not documenting the reason for a clone | Add `// clone required: <reason>` comment |
| Storing a `Timer` in a local variable | Keep it in a field or `Rc<RefCell<Option<Timer>>>` |
| Calling `unwrap()` on a `RwLock` | Use `unwrap_or_else(|p| p.into_inner())` for poison recovery |

---

## Reference File Index

| Pattern | Reference File | Lines |
|---------|---------------|-------|
| global UiState definition | `docs/master-password/app/src/ui/app.slint` | L43-252 |
| UI struct + initialization | `docs/master-password/app/src/ui/mod.rs` | L62-166 |
| Callback registration example | `docs/master-password/app/src/ui/mod.rs` | L630-711 |
| VecModel conversion | `docs/master-password/app/src/ui/mod.rs` | L459-482 |
| Timer usage example | `docs/master-password/app/src/ui/mod.rs` | L390-440 |
| AppContext state management | `docs/master-password/app/src/app_context.rs` | L190-310 |
| RwLock poison recovery | `docs/master-password/app/src/app_context.rs` | L292-310 |
| build.rs | `docs/master-password/app/build.rs` | L1-53 |

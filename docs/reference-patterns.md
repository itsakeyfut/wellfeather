# Slint + Rust 連携パターン集

> 参考プロジェクト: `docs/master-password/`
> 主要参照ファイル:
> - `app/src/ui/mod.rs` — Slint↔Rust 連携の中心
> - `app/src/app_context.rs` — 状態管理の実装
> - `app/build.rs` — ビルド設定

---

## 1. Slint `global` による UI状態の統一管理

全プロパティ・コールバックを1つの `global` コンポーネントにまとめる。
Rust からは `window.global::<UiState>()` で読み書きする。

```slint
// app.slint
export global UiState {
    // プロパティ (in-out = Rust/Slint双方向)
    in-out property <bool>   is_loading: false;
    in-out property <string> current_screen: "main";
    in-out property <string> status_message: "";
    in-out property <[RowData]> result_rows: [];

    // コールバック (Slint → Rust)
    callback run_query(string);
    callback cancel_query();
    callback connect(string);
    callback disconnect(string);
}
```

```rust
// Rust側: プロパティの読み書き
let ui = window.global::<UiState>();
ui.set_is_loading(true);
ui.set_status_message("実行中...".into());
let rows = ui.get_result_rows();  // 読み取り
```

**参照**: `app/src/ui/app.slint` L43-252、`app/src/ui/mod.rs` L169-207

---

## 2. コールバック登録の分割パターン

`register_*_callbacks()` 関数ごとに分割し、`UI::new()` で一括登録する。
`main.rs` や `new()` にロジックが肥大化しない。

```rust
// ui/mod.rs
pub struct UI {
    window: AppWindow,
}

impl UI {
    pub fn new(ctx: AppContext) -> Result<Self, AppError> {
        let window = AppWindow::new()?;

        Self::init_ui_state(&window, &ctx);
        // 関心ごとにコールバックを分割登録
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
        // ... コールバック登録
    }
}
```

**参照**: `app/src/ui/mod.rs` L89-166

---

## 3. クロージャ内のウィンドウ参照: `as_weak()` パターン

コールバックのクロージャにウィンドウを直接キャプチャすると循環参照になる。
必ず `as_weak()` で弱参照を取り、使用時に `upgrade()` する。

```rust
fn register_query_callbacks(window: &AppWindow, ctx: AppContext) {
    let ui = window.global::<UiState>();

    // NG: window を直接クロージャにキャプチャしない
    // ui.on_run_query(move |sql| { window.global::<UiState>()... });

    // OK: 弱参照を使う
    let window_weak = window.as_weak();
    ui.on_run_query(move |sql| {
        if let Some(window) = window_weak.upgrade() {
            window.global::<UiState>().set_is_loading(true);
        }
    });
}
```

**参照**: `app/src/ui/mod.rs` L641-711

---

## 4. コールバック内の `ctx.clone()` パターン

各コールバックは `AppContext` の所有権を必要とするため、クロージャに渡す前に `clone()` する。
`AppContext` は `Arc<RwLock<Inner>>` のラッパーなので clone はポインタコピーのみ。

```rust
fn register_query_callbacks(window: &AppWindow, ctx: AppContext) {
    let ui = window.global::<UiState>();

    // コールバックごとに clone
    {
        // clone required: callback closure needs owned ctx
        let ctx = ctx.clone();
        let window_weak = window.as_weak();
        ui.on_run_query(move |sql| {
            // ctx を使って処理
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

> **規約**: clone が必要な箇所には必ず `// clone required: <理由>` のコメントを付ける。
> （`app/src/ui/mod.rs` L641, L716 等）

---

## 5. `Arc<RwLock<Inner>>` + ポイズニング回復パターン

状態管理の中心となる `AppContext` は `Arc<RwLock<Inner>>` でスレッド安全に共有する。
パニック後のロックポイズニングから回復する `read()` / `write()` ヘルパーを必ず実装する。

```rust
// app_context.rs
#[derive(Clone)]
pub struct AppContext {
    inner: Arc<RwLock<AppContextInner>>,
}

impl AppContext {
    /// ロックポイズニングから回復しつつ読み取りロックを取得
    fn read(&self) -> RwLockReadGuard<'_, AppContextInner> {
        self.inner
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// ロックポイズニングから回復しつつ書き込みロックを取得
    fn write(&self) -> RwLockWriteGuard<'_, AppContextInner> {
        self.inner
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    // 外部からはメソッド経由のみアクセス (RwLock を直接触らせない)
    pub fn is_loading(&self) -> bool {
        self.read().is_loading
    }

    pub fn set_loading(&self, v: bool) {
        self.write().is_loading = v;
    }
}
```

**参照**: `app/src/app_context.rs` L190-310

---

## 6. `slint::VecModel` でリストをバインド

Slint のリスト (`[T]` プロパティ) には `VecModel` を使う。
ドメインモデル → UI用プレビュー型の変換メソッドをセットで実装する。

```rust
use std::rc::Rc;

// ドメインモデル → Slint UI型 の変換
fn rows_to_ui(rows: &QueryResult) -> Vec<crate::RowData> {
    rows.rows.iter().map(|r| crate::RowData {
        // clone required: Slint requires owned SharedString
        cells: r.iter().map(|c| match c {
            Some(v) => v.clone().into(),
            None    => slint::SharedString::default(),
        }).collect::<slint::ModelRc<_>>().into(),
    }).collect()
}

// UIに反映
let model = Rc::new(slint::VecModel::from(rows_to_ui(&result)));
ui_state.set_result_rows(model.into());
```

**参照**: `app/src/ui/mod.rs` L459-482, L452-453

---

## 7. `slint::Timer` の使い方

Slint の `Timer` はイベントループと同スレッドで動作する。
`TimerMode::Repeated` で繰り返し実行、`TimerMode::SingleShot` で1回実行。

```rust
use std::time::Duration;

// 繰り返しタイマー (例: ステータスバーの実行時間更新)
let timer = slint::Timer::default();
timer.start(
    slint::TimerMode::Repeated,
    Duration::from_millis(100),
    move || {
        // UIスレッドで実行される
        if let Some(window) = window_weak.upgrade() {
            // ...
        }
    },
);
// timer を drop するとタイマーが止まる → フィールドに保持して制御する

// 1回限りタイマー (例: デバウンス後の補完トリガー)
let timer = slint::Timer::default();
timer.start(
    slint::TimerMode::SingleShot,
    Duration::from_millis(300),
    move || {
        // 300ms後に1回だけ実行
    },
);
```

> **注意**: `Timer` を `drop` すると即座に停止する。
> フィールドや `Rc<RefCell<Option<Timer>>>` で保持して寿命を管理する。

**参照**: `app/src/ui/mod.rs` L390-440 (ホットキーポーリング), L1249-1326 (クリップボードカウントダウン)

---

## 8. `Rc<RefCell<>>` で UIスレッド内の可変状態を共有

複数のクロージャ間でUIスレッド内のみの可変状態を共有するとき、
`Arc<Mutex<>>` ではなく `Rc<RefCell<>>` を使う（Slintはシングルスレッド）。

```rust
// 例: デバウンスタイマーを複数のクロージャで共有
let debounce_timer: Rc<RefCell<Option<slint::Timer>>> = Rc::new(RefCell::new(None));

{
    let debounce_timer = debounce_timer.clone();
    let window_weak = window.as_weak();
    ui.on_text_changed(move |text| {
        // 前のタイマーをキャンセル (dropで停止)
        *debounce_timer.borrow_mut() = None;

        let window_weak = window_weak.clone();
        let text = text.to_string();
        let timer = slint::Timer::default();
        timer.start(
            slint::TimerMode::SingleShot,
            Duration::from_millis(300),
            move || {
                if let Some(window) = window_weak.upgrade() {
                    // 300ms後に補完を実行
                    trigger_completion(&window, &text);
                }
            },
        );
        *debounce_timer.borrow_mut() = Some(timer);
    });
}
```

**参照**: `app/src/ui/mod.rs` L1249-1326

---

## 9. 非同期処理 + UI更新: `invoke_from_event_loop`

> **この項目は master-password にはない wellfeather 固有のパターン。**
> master-password は同期処理のみだが、wellfeather はDB クエリが非同期のため必須。

Slint の UI 更新はイベントループスレッドからしか行えない。
tokio タスクから UI を更新するには `slint::invoke_from_event_loop` を使う。

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
            // UIスレッド外 (tokioランタイム) で実行
            let result = ctx.db.execute(&sql).await;

            // UIスレッドに戻って更新
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

> **注意**: `invoke_from_event_loop` のクロージャ内では `Rc` が使えない。
> VecModel の生成はクロージャ内で行う。

---

## 10. 初期 UI 状態の設定 (`init_ui_state`)

`UI::new()` でコールバック登録前に、現在の状態を UI に反映する初期化関数を用意する。

```rust
fn init_ui_state(window: &AppWindow, ctx: &AppContext) {
    let ui = window.global::<UiState>();

    // 状態から初期画面を決定
    ui.set_current_screen(ctx.initial_screen().into());

    // 各プロパティを設定
    ui.set_is_loading(false);
    ui.set_theme(ctx.theme().into());
    ui.set_font_size(ctx.font_size() as i32);
}
```

**参照**: `app/src/ui/mod.rs` L169-207

---

## 11. `build.rs` の設定

`.slint` ファイルを Rust コードにコンパイルするための最小設定。
変更検知パスを登録することでインクリメンタルビルドが効く。

```rust
// build.rs
fn main() {
    println!("cargo:rerun-if-changed=src/ui/app.slint");

    // components/ 配下の .slint ファイルも監視
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

Rust 側でコンポーネントを使えるようにするには:

```rust
// main.rs または lib.rs
slint::include_modules!();
```

**参照**: `app/build.rs`

---

## 12. Slint の型変換

Rust の文字列 ↔ Slint の `SharedString` は `.into()` で相互変換できる。

```rust
// Rust String / &str → SharedString
ui.set_message("hello".into());
ui.set_message(my_string.clone().into());

// SharedString → Rust String
let s: String = ui.get_message().to_string();

// i32 への変換 (Slint は整数を i32 で扱う)
ui.set_row_count(result.row_count as i32);
ui.set_font_size(config.font_size as i32);

// bool はそのまま
ui.set_is_loading(true);
```

---

## 13. エラー処理とUIへの表示

エラーはダイアログを使わず、`UiState` のエラープロパティ経由でインライン表示する。

```rust
match result {
    Ok(r)  => {
        ui.set_error_message("".into());  // エラーをクリア
        // 結果を反映
    }
    Err(e) => {
        tracing::error!("Query failed: {}", e);
        ui.set_error_message(e.to_string().into());
        ui.set_is_loading(false);
    }
}
```

---

## 14. よくあるミスと対策

| ミス | 対策 |
|------|------|
| ウィンドウを直接クロージャにキャプチャ | `as_weak()` + `upgrade()` を使う |
| UIスレッド外から直接プロパティ更新 | `invoke_from_event_loop` を使う |
| `Rc` を tokio タスク内で使う | クロージャ外で `Vec` に変換してから渡す |
| clone の理由を書かない | `// clone required: <理由>` コメントを付ける |
| `Timer` を局所変数に置く | フィールドまたは `Rc<RefCell<Option<Timer>>>` で保持 |
| RwLock の `unwrap()` | `unwrap_or_else(|p| p.into_inner())` でポイズニング回復 |

---

## 参照ファイル一覧

| パターン | 参照ファイル | 行 |
|---------|-------------|-----|
| global UiState 定義 | `docs/master-password/app/src/ui/app.slint` | L43-252 |
| UI 構造体・初期化 | `docs/master-password/app/src/ui/mod.rs` | L62-166 |
| コールバック登録例 | `docs/master-password/app/src/ui/mod.rs` | L630-711 |
| VecModel 変換 | `docs/master-password/app/src/ui/mod.rs` | L459-482 |
| Timer 使用例 | `docs/master-password/app/src/ui/mod.rs` | L390-440 |
| AppContext 状態管理 | `docs/master-password/app/src/app_context.rs` | L190-310 |
| RwLock ポイズニング回復 | `docs/master-password/app/src/app_context.rs` | L292-310 |
| build.rs | `docs/master-password/app/build.rs` | L1-53 |

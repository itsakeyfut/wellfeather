# Wellfeather — Testing Standards

## References

- [Rust Testing Guide](https://doc.rust-lang.org/book/ch11-00-testing.html)
- [proptest Book](https://proptest-rs.github.io/proptest/intro.html)
- [Criterion Book](https://bheisler.github.io/criterion.rs/book/)

---

## Philosophy

Test **behavior**, not implementation. A test that breaks only when observable behavior changes
is a good test. A test that breaks when you rename an internal field is not.

---

## Test Naming Convention

All test functions follow the pattern:

```
<feature>_should_<expected_result>
```

```rust
// ✅ Good names — describe what the system should do
fn connect_should_send_connected_event_on_success()
fn apply_limit_should_not_modify_dml_statements()
fn extract_statement_at_should_trim_whitespace_around_statement()
fn cache_load_should_fall_back_to_sqlite_after_restart()

// ❌ Bad names — describe implementation, not behavior
fn test_connect()
fn test_cache()
```

---

## Test Layers

### 1. Unit tests (primary)

Place unit tests in a `#[cfg(test)] mod tests { ... }` block inside the source file.
Each test exercises a single function or method in isolation.

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_statement_at_should_return_whole_string_when_no_semicolon() {
        let sql = "SELECT * FROM users";
        let result = extract_statement_at(sql, 5);
        assert_eq!(result, "SELECT * FROM users");
    }
}
```

### 2. Integration tests

Integration tests in `app/tests/` exercise multiple components together.
For library crates, integration tests that require a real DB use `#[ignore]`.

```rust
// crates/wf-db/src/drivers/pg.rs
#[cfg(test)]
mod tests {
    #[tokio::test]
    #[ignore] // requires real PostgreSQL instance
    async fn connect_should_succeed_with_real_postgres() { ... }
}
```

SQLite in-memory is used for all DB-dependent tests that must run in CI.

```rust
#[tokio::test]
async fn service_should_return_query_result() {
    let conn = DbConnection {
        db_type: DbType::Sqlite,
        database: None, // ":memory:"
        ..Default::default()
    };
    let service = DbService::new();
    service.connect(&conn).await.unwrap();
    // ...
}
```

### 3. Property-based tests (proptest)

Use proptest for logic that must hold for **arbitrary inputs** — not just the cases you
thought of. Appropriate targets:

- SQL parser / analyzer (arbitrary SQL strings)
- Completion engine (arbitrary prefixes)
- CSV / JSON export (arbitrary row data)
- Crypto roundtrip (arbitrary passwords and plaintexts)

```toml
# Cargo.toml for the relevant crate
[dev-dependencies]
proptest = "1"
```

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn export_csv_should_roundtrip_any_cell_value(
        value in "[ -~]{0,100}"  // printable ASCII
    ) {
        let result = QueryResult {
            columns: vec!["col".into()],
            rows: vec![vec![Some(value.clone())]],
            ..Default::default()
        };
        let csv = result_to_csv_bytes(&result);
        // CSV must contain the value (possibly quoted)
        prop_assert!(String::from_utf8_lossy(&csv).contains(&value.replace('"', "\"\"")));
    }

    #[test]
    fn extract_statement_should_never_panic(
        sql in ".*",
        cursor in 0usize..=200
    ) {
        // Must not panic regardless of input
        let cursor = cursor.min(sql.len());
        let _ = extract_statement_at(&sql, cursor);
    }
}
```

### 4. Criterion benchmarks

See `docs/rules/perf.md` for benchmark placement and structure.
Benchmarks are not required for every function, only for documented performance-sensitive paths.

---

## What to Test per Crate

### `wf-db`

| Target | Tests |
|--------|-------|
| `DbService::connect` | event on success, `ConnectError` on bad URL |
| `DbService::execute` | returns `QueryResult`, handles `QueryError` |
| `DbService::execute_with_cancel` | result when not cancelled, `Cancelled` when token fires |
| `DbService::disconnect` | removes pool, noop for unknown ID |
| Pool URL construction | all three DB types, connection string vs field mode |
| `DbError` display | all variants display correctly |
| SQLite driver | select/insert/update/delete/null/metadata |
| PG/MySQL drivers | `#[ignore]`, real DB only |

### `wf-config`

| Target | Tests |
|--------|-------|
| `ConfigManager` | roundtrip save/load, returns default when absent |
| `crypto` | encrypt/decrypt roundtrip, fails on tampered ciphertext |
| `Config` model | deserializes from minimal/full TOML, roundtrip serde |

### `wf-query`

| Target | Tests |
|--------|-------|
| `extract_statement_at` | multiple statements, trailing semicolons, edge positions |
| `extract_selection` | exact byte range, reversed range, clamp |
| `format_sql` | uppercase keywords, indent, empty input |
| `result_to_csv_bytes` | BOM, header, null handling, comma escaping |
| `result_to_json_bytes` | null → JSON null, numeric coercion |

### `wf-completion`

| Target | Tests |
|--------|-------|
| `CompletionEngine::complete` | keyword/table/column/join contexts, prefix filtering, alias resolution |
| `Parser::parse_context` | all context types, dot notation, aliases, case insensitivity |
| `MetadataCache` | store/load roundtrip, file persistence, preload |
| `CompletionService` | returns candidates, handles empty metadata, debounce |
| proptest | arbitrary prefix inputs do not panic, prefix filtering is stable |

### `wf-history`

| Target | Tests |
|--------|-------|
| `HistoryService::insert` + `recent` | roundtrip, limit respected, empty state |

### `app`

| Target | Tests |
|--------|-------|
| `AppController` events | each Command produces the expected Event(s) |
| `apply_limit` | appends LIMIT, case insensitive, skips DML, already-limited queries |
| Session restore | save/load connection ID, save/load query, roundtrip |
| UI utility functions | `filter_rows`, `sort_rows`, `cells_to_tsv`, `build_sidebar_tree`, etc. |

---

## What NOT to Test

- Visual appearance of Slint components (pixel colors, layout dimensions)
- Slint property bindings directly (these are UI contracts, not logic)
- Third-party crate internals (`sqlx`, `slint`, `tokio`)
- Generated code (Slint macro output)
- Happy-path-only trivial getters/setters unless they contain logic

---

## Async Tests

Use `#[tokio::test]` for async test functions.

```rust
#[tokio::test]
async fn run_query_should_send_query_started_then_finished() {
    // ...
}
```

Do not use `tokio::test(flavor = "multi_thread")` unless the test specifically requires
multiple threads. Single-threaded is faster and more deterministic.

---

## Test Helpers

Define shared test utilities as functions in a `#[cfg(test)]` module within the relevant
file, or in `tests/common/` for integration tests.
Do not use a global test fixture framework — Rust's module system is sufficient.

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn make_metadata() -> DbMetadata {
        DbMetadata {
            tables: vec![
                TableMeta { name: "users".into(), columns: vec![
                    ColumnMeta { name: "id".into(), col_type: "bigint".into(), .. },
                    ColumnMeta { name: "email".into(), col_type: "varchar(255)".into(), .. },
                ]},
            ],
            ..Default::default()
        }
    }

    #[test]
    fn complete_should_return_columns_for_users_table() {
        let engine = CompletionEngine::new();
        let metadata = make_metadata();
        let items = engine.complete_with_metadata("SELECT  FROM users", 7, &metadata);
        assert!(items.iter().any(|i| i.label == "id"));
        assert!(items.iter().any(|i| i.label == "email"));
    }
}
```

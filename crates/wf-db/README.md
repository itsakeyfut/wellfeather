# wf-db

Database connection pool and query execution layer for [wellfeather](../../README.md).

Provides enum-dispatched connection pooling over PostgreSQL, MySQL, and SQLite via `sqlx`,
along with `DbService` for query execution and schema fetching.

## Responsibilities

- `pool/` — `DbPool` enum wrapping `PgPool`, `MySqlPool`, `SqlitePool`
- `service/` — `DbService`: execute queries, fetch table/column schema
- `drivers/` — per-database driver helpers (`pg`, `my`, `sqlite`)
- `models/` — shared data types (`Row`, `Column`, `SchemaInfo`, etc.)
- `error/` — typed error variants via `thiserror`

## Usage

This crate is an internal library used only by the `app` binary.
It is not intended for publication on crates.io.

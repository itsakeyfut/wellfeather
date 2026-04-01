# wf-history

SQLite-backed query execution history for [wellfeather](../../README.md).

Persists `QueryExecution` records (SQL text, execution time, row count, error) to a local
SQLite database via `HistoryService`, and provides paginated retrieval.

## Responsibilities

- `service/` — `HistoryService`: save and query `QueryExecution` records via SQLite

## Usage

This crate is an internal library used only by the `app` binary.
It is not intended for publication on crates.io.

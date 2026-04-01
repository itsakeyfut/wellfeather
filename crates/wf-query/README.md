# wf-query

SQL cursor analysis, formatting, and export for [wellfeather](../../README.md).

Provides utilities for analyzing SQL at the current cursor position (for completion hints),
formatting SQL text, and exporting query results to CSV or JSON.

## Responsibilities

- `analyzer/` — cursor-position SQL analyzer: detect context (table name, column list, etc.)
- `formatter/` — SQL formatter / pretty-printer
- `export/` — CSV and JSON export from query result rows

## Usage

This crate is an internal library used only by the `app` binary.
It is not intended for publication on crates.io.

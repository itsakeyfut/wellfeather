# wf-completion

SQL completion engine with metadata cache and debounce for [wellfeather](../../README.md).

Provides real-time SQL autocompletion by combining a debounced `CompletionService`,
an in-memory + SQLite-backed `MetadataCache`, and a `CompletionEngine` that ranks candidates.

## Responsibilities

- `service/` — `CompletionService`: debounced completion requests, tokio task management
- `cache/` — `MetadataCache`: in-memory cache with SQLite flush for schema metadata
- `engine/` — `CompletionEngine`: candidate generation and ranking
- `parser/` — lightweight SQL token parser for completion context

## Usage

This crate is an internal library used only by the `app` binary.
It is not intended for publication on crates.io.

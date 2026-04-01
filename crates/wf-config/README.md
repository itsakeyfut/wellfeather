# wf-config

Configuration management and AES-256-GCM password encryption for [wellfeather](../../README.md).

Handles loading and saving `config.toml`, storing connection profiles, and encrypting
database passwords at rest using AES-256-GCM.

## Responsibilities

- `manager/` — `ConfigManager`: load/save `config.toml` from the OS config directory
- `models/` — `AppConfig`, `ConnectionProfile`, and related types
- `crypto/` — AES-256-GCM encrypt/decrypt for stored passwords

## Usage

This crate is an internal library used only by the `app` binary.
It is not intended for publication on crates.io.

# v0.7.0 — Polish, Settings, and Export

> **Theme**: Complete the remaining MVP features and make the app fit for daily use.
> **Prerequisite**: v0.6.0

---

## Goal

Implement export, theming, font configuration, session restore, menu bar, localization, and logging
to finish wellfeather as a daily-use database client.

---

## Exit Criteria

- [ ] Query results can be exported as a CSV file
- [ ] Query results can be exported as a JSON file
- [ ] Dark / light theme can be toggled (saved to config, persists across restarts)
- [ ] `font_family` / `font_size` from `config.toml` are applied to the editor and result table
- [ ] The previous query is restored in the editor after restarting
- [ ] Major operations (export, theme toggle, etc.) are accessible from the menu bar
- [ ] Both English and Japanese locales are fully supported (Slint .po + rust-i18n .yml)
- [ ] Structured logging works (`RUST_LOG=debug` prints debug output)

---

## Scope

| Category | Content |
|----------|---------|
| CSV export | QueryResult → CSV conversion + file save |
| JSON export | QueryResult → JSON conversion + file save |
| Export UI | File save dialog for path selection |
| Theme switching | Dark/light toggle + config.toml persistence |
| Font config | Apply font settings from config.toml to editor and result table |
| Session restore | Restore previous query text in editor (extends v0.2.0 connection restore) |
| Menu bar | File and Settings menu implementation |
| Localization | Slint .po files (en/ja) + rust-i18n .yml files, LocalizedMessage trait on error types |
| Logging | tracing-subscriber setup + RUST_LOG support |

---

## Out of Scope

- Font settings UI (config.toml direct edit only; UI in v1.3.0)
- Startup time optimization (v1.0.0)
- Cross-platform testing (v1.0.0)

---

## Key Risks

- Slint does not provide a native file save dialog — add the `rfd` (Rust File Dialog) crate
- Theme switching uses Slint color globals (`global ThemeColors`) — verify color propagation to all components including `TextInput` and `ListView`
- Very long query strings (e.g. > 10KB) must still persist correctly in `config.toml`
- CSV encoding: output UTF-8 BOM (`\xEF\xBB\xBF`) for Excel compatibility on Windows

---

## Task List

See `docs/roadmap/tasks/v0-7-0.md` for details.

| Task ID | Title | Issue |
|---------|-------|-------|
| T071 | CSV export of query results | #45 |
| T072 | JSON export of query results | #46 |
| T073 | Dark/light theme switching with ThemeColors global | #47 |
| T074 | Font family/size configuration applied to editor and result table | #48 |
| T075 | Session restore: persist and reload editor query text | #49 |
| T076 | Custom menu bar (File / Edit / Query / Settings) | #50 |
| T077 | tracing setup: structured logging with RUST_LOG control | #51 |
| — | Localization support (Slint i18n + rust-i18n, en/ja) | #79 |

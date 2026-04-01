# v1.0.0 — MVP Release (Stability and Performance Verification)

> **Theme**: Verify MVP quality and prove the core value proposition with numbers.
> **Prerequisite**: v0.7.0 (all MVP features implemented)

---

## Goal

Stabilize all MVP features implemented through v0.7.0 and measure the core value proposition —
startup < 1 second, low memory usage.
Release with confirmed builds and basic functionality verified on Windows and macOS
(Linux verified separately; full Linux packaging deferred to v2.0.0).

---

## Exit Criteria

- [ ] **Startup time < 1 second** (release build, measured on Windows and macOS)
- [ ] Memory usage measured and recorded (compare against DBeaver / TablePlus)
- [ ] Build passes and basic functionality verified on Windows and macOS
- [ ] All `cargo test` tests pass (unit + integration)
- [ ] CI pipeline is in place (fmt-check, clippy, test)
- [ ] README.md contains setup instructions and basic usage
- [ ] Platform abstraction layer in place (`app/src/platform/`, path DI, DPI awareness)
- [ ] Windows MSIX and macOS DMG packages can be built via `cargo x package`

---

## Performance Targets

| Metric | Target | How to Measure |
|--------|--------|---------------|
| Startup time (cold) | < 1 second | `time ./wellfeather` |
| Startup time (warm) | < 0.5 seconds | same (after config loaded) |
| Idle memory | < 50 MB | OS process monitor |
| Memory with 100k rows | < 200 MB | same |

---

## Scope

| Category | Content |
|----------|---------|
| Performance measurement | Startup time and memory profiling, optimization if needed |
| Cross-platform | Build verification and path / shortcut testing on Windows and macOS |
| Test coverage | Unit and integration test additions, CI pipeline setup |
| Documentation | README.md, installation steps, basic usage |
| Release build | `[profile.release]` optimization settings (LTO, strip, panic=abort) |
| Platform abstraction | `app/src/platform/` — data/config/cache dirs, DPI awareness, dark mode detection |
| Distribution packaging | Windows MSIX + macOS DMG via `cargo x package` with GitHub Actions release workflow |
| Bug fixes | Fix issues discovered during v0.1.0–v0.7.0 development |

---

## Key Risks

- Cross-platform: keyboard shortcut differences (Ctrl vs Cmd on macOS)
- Cross-platform: `directories` crate path differences (Linux XDG compliance)
- Performance: sqlx initialization cost may be significant — consider lazy initialization
- Performance: Slint initial render cost (font loading, etc.) may need optimization

---

## Task List

See `docs/roadmap/tasks/v1-0-0.md` for details.

| Task ID | Title | Issue |
|---------|-------|-------|
| T101 | Startup time measurement and optimization (< 1 second) | #52 |
| T102 | Memory usage measurement (idle < 50 MB, 100k rows < 200 MB) | #53 |
| T103 | Cross-platform build verification (Windows + macOS) | #54 |
| T104 | Test coverage audit + GitHub Actions CI pipeline | #55 |
| T105 | README.md and user documentation | #56 |
| T106 | Release build profile optimization (LTO, strip, panic=abort) | #57 |
| T107 | v0.x bug fixes and clippy clean-up | #58 |
| — | Platform abstraction layer (data/config/cache dirs, DPI awareness, dark mode) | #80 |
| — | Distribution packaging: Windows MSIX + macOS DMG (MVP release) | #81 |

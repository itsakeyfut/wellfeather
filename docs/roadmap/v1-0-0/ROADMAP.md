# v1.0.0 — Release Preparation (Stability, Performance, and Packaging)

> **Theme**: Verify quality and ship the first public release.
> **Prerequisite**: v0.15.0 (all v0.x.0 milestones complete)

---

## Goal

Stabilize all features implemented through v0.15.0 and measure the core value proposition —
startup < 1 second, low memory usage. Produce release packages for Windows, macOS, and Linux
and ensure CI is in place before the first public release.

> **Note**: The exact feature scope that constitutes "v1.0.0" is intentionally not fixed.
> All v0.x.0 milestones must be complete before this milestone begins.
> This milestone contains only release-preparation tasks (performance, packaging, docs, CI).

---

## Exit Criteria

- [ ] **Startup time < 1 second** (release build, measured on Windows and macOS)
- [ ] Memory usage measured and recorded (compare against DBeaver / TablePlus)
- [ ] Build passes and basic functionality verified on Windows, macOS, and Linux
- [ ] All `cargo test` tests pass (unit + integration)
- [ ] CI pipeline is in place (fmt-check, clippy, test) running on all 3 platforms
- [ ] README.md contains setup instructions and basic usage
- [ ] Platform abstraction layer in place (`app/src/platform/`, path DI, DPI awareness)
- [ ] Windows MSIX and macOS DMG packages can be built via `cargo x package`
- [ ] Linux packages built via `cargo x package --platform linux` (AppImage / .deb)

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
| Cross-platform | Build verification and path / shortcut testing on all 3 platforms |
| Test coverage | Unit and integration test additions, CI pipeline setup |
| Documentation | README.md, installation steps, basic usage, keyboard shortcuts |
| Release build | `[profile.release]` optimization settings (LTO, strip, panic=abort) |
| Platform abstraction | `app/src/platform/` — data/config/cache dirs, DPI awareness, dark mode detection |
| Distribution packaging | Windows MSIX + macOS DMG + Linux AppImage via `cargo x package` |
| Bug fixes | Fix issues discovered during v0.x.0 development |

---

## Task List

See `docs/roadmap/tasks/v1-0-0.md` for details.

| Task ID | Title | Issue |
|---------|-------|-------|
| T101 | Startup time measurement and optimization (< 1 second) | #52 |
| T102 | Memory usage measurement (idle < 50 MB, 100k rows < 200 MB) | #53 |
| T103 | Cross-platform build verification (Windows + macOS + Linux) | #54 |
| T104 | Test coverage audit + GitHub Actions CI pipeline | #55 |
| T105 | README.md and user documentation | #56 |
| T106 | Release build profile optimization (LTO, strip, panic=abort) | #57 |
| T107 | v0.x bug fixes and clippy clean-up | #58 |
| — | Platform abstraction layer (data/config/cache dirs, DPI awareness, dark mode) | #80 |
| — | Distribution packaging: Windows MSIX + macOS DMG (MVP release) | #81 |

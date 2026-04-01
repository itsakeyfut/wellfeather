# v2.0.0 — Analysis and Optimization Hints (Phase 3)

> **Theme**: Evolve wellfeather from a tool you "just use" to one that helps you improve.
> **Prerequisite**: v1.4.0

---

## Goal

Implement lightweight analysis and performance hints so wellfeather can suggest improvements
for missing indexes and slow queries.

---

## Exit Criteria

- [ ] Query result metadata (row count, execution plan) is analyzed and improvement hints are displayed
- [ ] WHERE clause usage on unindexed columns is detected and a hint is shown
- [ ] DB passwords can be stored in the OS keychain (Windows Credential Manager / macOS Keychain)
- [ ] Linux distribution packages (AppImage / .deb / .rpm) can be built via `cargo x package --platform linux`

---

## Task List

See `docs/roadmap/tasks/v2-0-0.md` for details.

| Task ID | Title | Issue |
|---------|-------|-------|
| T201 | Lightweight performance analysis engine with hint generation | #72 |
| T202 | Index advisor: missing index detection and suggestion | #73 |
| T203 | OS keychain integration for password storage | #74 |
| — | Linux distribution packaging (AppImage / .deb / .rpm) | #82 |

# v1.1.0 — Editor Enhancement (Syntax Highlighting + Multi-tab)

> **Theme**: Make the editor richer. Start of Phase 2.
> **Prerequisite**: v1.0.0

---

## Goal

Implement syntax highlighting and multi-tab support so users can write multiple queries in parallel.

---

## Exit Criteria

- [ ] SQL keywords, string literals, and comments are color-coded in the editor
- [ ] Multiple editor tabs can be opened, each managing an independent query
- [ ] Tabs share the active connection (one connection per app)
- [ ] Tabs can be added, closed, and renamed

---

## Key Risks

- **Syntax highlighting in Slint**: Slint's `TextInput` does not natively support per-token styling. A custom rendering approach (canvas overlay or custom `Path`/`Text` elements) may be required. Complete the investigation spike (#84) before starting implementation
- Multi-tab support requires a state management redesign: which tab is active, and each tab's independent query text and result

---

## Task List

See `docs/roadmap/tasks/v1-1-0.md` for details.

| Task ID | Title | Issue |
|---------|-------|-------|
| — | Slint syntax highlighting: technical investigation spike | #84 |
| T111 | SQL syntax highlighting in editor | #59 |
| T112 | Multi-tab editor UI | #60 |
| T113 | Per-tab state management (TabState in AppState) | #61 |

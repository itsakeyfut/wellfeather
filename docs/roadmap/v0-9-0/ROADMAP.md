# v0.9.0 — Keyboard Enhancement (Vim Mode + Command Palette)

> **Theme**: Complete wellfeather's keyboard-centric differentiation.
> **Prerequisite**: v0.8.0

---

## Goal

Implement Vim keybindings and a command palette to complete the experience of
"operating a database without a mouse" — wellfeather's primary differentiator.

---

## Exit Criteria

- [ ] Normal / Insert / Visual modes work in the editor
- [ ] Vim basic operations work (movement, deletion, yank, paste, search)
- [ ] `Ctrl+K` opens the command palette
- [ ] Connection switching, query execution, and settings changes are accessible from the command palette
- [ ] Fuzzy search filters the command palette

---

## Key Risks

- Vim mode is complex — consider reusing an existing Rust Vim crate (`vim-like`, etc.)
- Making Slint's `TextInput` Vim-compatible requires deep understanding of its text manipulation API; investigate before implementation
- The command palette is implemented with Slint's `PopupWindow`; fuzzy matching logic needs a dedicated crate (`fuzzy-matcher`, etc.)

---

## Task List

See `docs/roadmap/tasks/v0-9-0.md` for details.

| Task ID | Title | Issue |
|---------|-------|-------|
| T121 | Vim keybindings: Normal/Insert/Visual mode | #62 |
| T122 | Command palette UI with fuzzy search (Ctrl+K) | #63 |
| T123 | Command palette action registry | #64 |

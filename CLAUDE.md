# Rule
## Understand what I want to achieve on this project
- Read docs/specification.md at first
- Grasp of what this project aims at

## Before writing any code
- Read docs/architecture.md to understand module structure and design decisions
- Read docs/reference-patterns.md for Slint + Rust integration patterns
  - This document contains ALL patterns to follow when writing Slint↔Rust code
  - Follow these patterns strictly: weak references, callback registration, VecModel, invoke_from_event_loop, etc.

## NOTE
- Do not generate codes before we decide on specific specifications and architecture
- Do not decide a simple way to intrude
- Decide specific specifications and architecture first
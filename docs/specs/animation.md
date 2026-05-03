# Animation Specification

## Philosophy

wellfeather is a keyboard-centric editor tool. Animations must **never interrupt the user's
workflow**. The guiding rule is:

> If the user triggered an action, the result should appear immediately.

- Active operations (modal open/close, sidebar expand/collapse) are **instant** — no transition
- Passive feedback (button hover, checkbox toggle) use short transitions that do not block interaction
- The `Animation` global and `reduce-motion` infrastructure are preserved for future phases
  (ER diagrams, graph visualisations) where richer motion will be appropriate

---

## Design Tokens

Animation constants are centralised in the `Animation` global in `theme.slint`.
Components must never hard-code duration or easing values — always reference this global.

```slint
export global Animation {
    in-out property <bool> reduce-motion: false;

    // ── Duration tokens ──────────────────────────────────────────────────────
    out property <duration> instant:       reduce-motion ? 0ms :  80ms;  // (reserved)
    out property <duration> fast:          reduce-motion ? 0ms : 120ms;  // (reserved)
    out property <duration> feedback:      reduce-motion ? 0ms : 150ms;  // hover, checkbox
    out property <duration> standard:      reduce-motion ? 0ms : 200ms;  // (reserved)
    out property <duration> enter:         reduce-motion ? 0ms : 160ms;  // (reserved)
    out property <duration> stagger-step:  reduce-motion ? 0ms :  30ms;  // (reserved)

    // ── Easing tokens ────────────────────────────────────────────────────────
    out property <easing> enter-ease:   ease-out;
    out property <easing> exit-ease:    ease-in;
    out property <easing> value-ease:   ease-in-out;
    out property <easing> linear-ease:  linear;
}
```

---

## Active Animations

| Component | Property | Duration | Easing | Notes |
|-----------|----------|----------|--------|-------|
| ToolbarButton hover | `background` | 150ms | ease-out | Active/inactive state change |
| ActionButton hover overlay | `opacity` | 150ms | ease-out | Subtle hover tint |
| CheckRow checkbox | `background`, `border-color` | 150ms | ease-in-out | Checked state transition |
| CheckRow check icon | `opacity` | 120ms | ease-in-out | Icon fade-in |
| Loading spinner | `ProgressIndicator` (indeterminate) | — | — | std-widgets built-in |

---

## Intentionally Removed

| Component | Reason |
|-----------|--------|
| Modal enter/exit | Instant open/close is less disruptive in an editor context |
| Sidebar expand/collapse | Immediate response is expected on key press or click |
| Button press scale | Too app-like for an editor tool |
| Button ripple effect | Too Material Design; inconsistent with editor aesthetic |

---

## Reserved for Future Phases

The `Animation` tokens and `reduce-motion` support are intentionally kept for:

- **ER diagram**: node drag, edge routing, pan/zoom transitions
- **Graph visualisations**: data series enter/exit, tooltip animations

When those features are implemented, each new animation must follow the `reduce-motion` pattern
so users can opt out of all motion at once.

---

## Reduce Motion

Stored in `config.toml` and toggled via View > Reduce Motion.

```toml
[appearance]
reduce_motion = false
```

All `Animation` duration tokens are conditioned on `reduce-motion`:

```slint
out property <duration> feedback: reduce-motion ? 0ms : 150ms;
```

Every new animation added to the codebase must follow this pattern.

---

## Slint Constraints and Workarounds

| Constraint | Workaround |
|------------|------------|
| `cubic-bezier` spring syntax not yet stable | Use `ease-out` as a placeholder; mark with a comment |
| No animate-completion callback | Use a `Timer` set to the exit duration before hiding the element |
| Elements created by `if` cannot animate on entry | Use `init =>` to set the target value after creation |
| No `transform: scale` | Approximate via `width`/`height` percentage change + `x`/`y` offset correction |
| No native list stagger support | Assign a stagger index from Rust; apply delayed `opacity` updates per item |

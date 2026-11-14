# iced_widget patch (Document2Voice)

Vendored from crates.io `iced_widget` **0.14.2** (for iced 0.14).

## Change

`src/overlay/menu.rs` — `Overlay::layout`:

- **Upstream:** open menu above or below the pick list, whichever has more space.
- **Here:** always open **below** the control; clamp height to remaining space so
  long menus scroll instead of flipping upward.
- Side effect: a pick list near the bottom of the window can open with a very short menu.

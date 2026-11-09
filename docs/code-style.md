# Code style — ghit

Rust edition 2021. `cargo build` must be warning-free; warnings are errors
in spirit. Fix the code, never silence the lint.

## Shape of the code

- Small functions with one job: fetch in `gh.rs`, pure transforms in
  `model.rs`, state transitions as `App` methods or free functions in
  `main.rs`, rendering as `render_*` functions.
- Prefer exhaustive handling: every user request lists ALL of its behaviors
  (happy path, error, empty, boundary). A happy-path-only patch is unfinished.
- No dead code. No commented-out blocks. If a helper loses its last caller,
  delete it.

## Rendering

- Colors only through `theme::` functions (`ink()`, `paper()`, `amber()`,
  …). A hardcoded `Color::` in a widget is a bug — it survives theme toggles.
- Selection is a marker (`▌`, `◆`) plus bold, never a full-width color block.
- Fixed card heights: collapsed 3 lines, expanded 6 (`BASE_CARD_H` /
  `EXPANDED_CARD_H`). Mouse hit areas are derived from the same constants
  via `card_height()`.
- `Frame::set_cursor_position` takes absolute cells; recompute it whenever
  the bar layout changes.

## State and IO

- The event loop never blocks on the network. Fetch on a thread, deliver via
  channel, apply in a `poll_*` step each frame.
- Cache reads tolerate absence: corrupt or expired entries mean refetch,
  never a crash.
- `gh` failures surface as status-line messages, not panics.

## Tests

- Pure logic gets unit tests in its module (`model.rs`, `cache.rs`, `theme.rs`).
- Anything the user can see gets a headless frame test in `render_smoke`
  (`TestBackend`, 120×40): render, assert on the joined cell text.
- Capture rules for tests: `body_rows` and hit areas are populated during
  `render`, so drawing a frame is enough — no manual capture calls.
- Name tests after the behavior: `space_toggles_expansion_and_move_clears_it`.

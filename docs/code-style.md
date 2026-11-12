# Code style — ghit

Rust edition 2021. `cargo build` must be warning-free; warnings are errors
in spirit. Fix the code, never silence the lint.

## Principles

What good code looks like here:

- Data first: types in `model.rs` carry the shape; behavior follows.
- Functional core, imperative shell: `model`/`core` stay pure; IO
  lives in `app`/`github.rs` behind channels.
- Parse, don't validate: check inputs once at the boundary, then
  trust the types.
- YAGNI: no hooks, frameworks, or helpers for imagined work.
- Rule of three: first copy is fine, second is a note, third lifts
  into a shared helper.
- Delete beats abstract: a helper left with one caller gets inlined
  and removed.

## Shape of the code

- Small functions with one job: fetch over HTTPS behind the `github.rs`
  facade (`api.rs` + `api/{client,sections,graph}.rs`/`cache.rs`;
  `gh.rs` is deprecated board-only),
  pure domain transforms in `model.rs`,
  pure business rules in `core.rs`, state and the event loop in `app.rs`,
  input mapping in `input.rs`, rendering as `render_*` functions in `ui.rs`.
- Layer discipline: `ui`/`input` never import `api`/`gh`/`cache`;
  `model`/`core` never import ratatui or crossterm. New network paths go
  through `github.rs`.
- Prefer exhaustive handling: every user request lists ALL of its behaviors
  (happy path, error, empty, boundary). A happy-path-only patch is unfinished.
- No dead code. No commented-out blocks. If a helper loses its last caller,
  delete it (see `docs/dead-code.md` and the `dead-code-sweep` skill).

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

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

## Files and modules

- Module paths use `foo.rs` plus a `foo/` folder for children
  (`api.rs` + `api/client.rs`). Never use `foo/mod.rs`. The Book calls
  `foo.rs` the idiomatic form and `mod.rs` the older style. [Book 7.5]
- One concern per file. A file name says what the file owns. Do not make
  `utils.rs`, `helpers.rs`, `types.rs` or `common.rs`: they collect
  unrelated code and only grow.
- Size budget (project rule). When a file passes about 800 lines without
  its tests, or about 1500 lines in total, split it before you add more.
  These files are over budget today: `ui.rs`, `app.rs`, `model.rs` and
  `input.rs`. When you change one of them, do not add a new concern to it.
  Split it in a separate `refactor:` change instead.
- How to split. Cut along a path seam: a group of functions that call each
  other, plus their tests. Do not cut by item kind (all structs here, all
  functions there). The parent file stays the module root. It re-exports
  only the items that callers outside the module use, so the callers do
  not change. Use #124 (the `api.rs` split) as the model.
- Tests stay in the same file as the code they test, in
  `#[cfg(test)] mod tests`. When code moves, its tests move with it.
- Item order in a file: `mod` declarations, then `use`, then re-exports.
  After that, types (`struct`, `enum`) come before `impl` blocks and
  functions. Public items come before private ones. [r-a: order of items,
  order of imports]
- Imports: put `std`, external crates and `crate::` in separate groups,
  with a blank line between groups. Prefer `use crate::foo::bar` to
  `use super::bar`. The exception is `use super::*` in a test module.
  Do not write a glob import such as `use MyEnum::*`. [r-a: import style]
- Visibility: use the narrowest level that compiles. Use `pub(super)` for
  helpers shared inside one module tree. Use `pub(crate)` for items that
  other modules use. Add a re-export only at a facade or module root, and
  only for an item that something outside the module actually uses. Every
  `pub use` you add widens the API. [r-a: scale of changes]

## Naming

- Casing follows RFC 430: `snake_case` for modules, functions and locals.
  Use `UpperCamelCase` for types, traits and variants, and
  `SCREAMING_SNAKE_CASE` for consts and statics. An acronym counts as one
  word: `GhHost`, not `GHHost`. [API C-CASE]
- Getters have no `get_` prefix: `cursor()`, not `get_cursor()`.
  Conversions follow these prefixes. `as_` is cheap and borrows. `to_` is
  costly. `into_` consumes the value. [API C-GETTER, C-CONV]
- Do not repeat the container in a name. Inside `DiffView`, a field is
  `cursor`, not `diff_cursor`. In `api`, a type is `Row`, not `ApiRow`.
- Locals get plain, full names. By default, name a local after its type in
  lowercase (`board: BoardData`). Avoid single letters except in short
  closures and loop indices. [r-a: variable naming]
- Use American spelling (`color`, `behavior`). Use one word for one idea
  across the crate. If it is `pending` in one struct, do not call it
  `in_flight` in another. [r-a: variable naming; API C-WORD-ORDER]

## Structs and state

- Group fields by owner. When a struct grows past about 20 fields, or holds
  two concerns that do not change together, move each concern into a
  sub-struct. `App` (#130) is the model: `app.diff.cursor`, not
  `app.diff_cursor`.
- Prefer `#[derive(Default)]` to a `new()` that takes no arguments. Write
  `new()` only when a field has no sensible default, such as an `mpsc`
  channel pair. Do not invent placeholder values just to make `Default`
  work. [r-a: constructors]
- Make a field public if any value is valid for it. If a field has an
  invariant, keep it private and give it a getter. Do not write setters.
  [r-a: getters & setters]
- Take borrowed views as parameters: `&str` over `&String`, `&[T]` over
  `&Vec<T>`. [r-a: useless types]

## Functions

- Return early. Handle the error or the empty case first, then write the
  main path without extra nesting. [r-a: early returns]
- Function size: aim for 100 lines or fewer, and 7 parameters or fewer.
  These are Clippy's default limits for `too_many_lines` and
  `too_many_arguments`. Treat them as a sign to split, not as a hard rule.
- Do not add a `bool` or `Option` parameter that every caller passes as a
  literal. Split the function in two. If a function would need more than
  three `bool` parameters, pass an `enum` or a small struct instead.
  [r-a: prefer separate functions over parameters]
- Do not create a helper function used in only one place, unless it needs
  `return` or `?`. Use a block instead. [r-a: helper functions]

## Comments

- A comment says why, not what. Write it as full sentences that start with
  a capital letter and end with a period. [Style Guide: comments]
- Put `///` on an item when its name and type do not explain it. Put the
  doc comment directly above that item, and above any attributes on it.
- Never write comments about the edit itself. Do not write "moved to X",
  "renamed from Y", "now lives in Z", "previously", "new:" or "was:".
  Git history records changes. A comment must stay true after the commit.
- No commented-out code. No `TODO` without an issue number.
- Error strings start in lowercase and do not end with a period (for
  example, "comment is empty: write something first"). They show on the
  status line. [r-a: error handling trivia]

## Formatting

- The Style Guide limit is 100 characters per line. [Style Guide] The
  codebase does not run `rustfmt` today, and about 360 lines are longer
  than that. Keep lines you add or change at 100 characters or less. Do
  not reformat lines you did not touch: a mass reformat hides real changes
  in review.

## Refactors

- A refactor changes structure only. It does not change behavior, text,
  keys or snapshots. Prove it: the tests pass without edits to their
  expected values.
- Make the smallest diff that does the job. Do not rename, reorder or
  restyle code next to it. Send that as its own change. [r-a: style]
- Leave no scaffolding. Do not keep re-export shims "for compatibility", do
  not add `#[allow(...)]` to hide a leftover, and do not keep wrappers whose
  only job is to keep an old name alive.

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
- No `#[should_panic]` tests: assert on the `Err` or `None` value instead.
  No `#[ignore]` tests: if a test shows a known bug, assert today's wrong
  result and link the issue. [r-a: should_panic, ignore]

## Sources

- [Book 7.5] https://doc.rust-lang.org/book/ch07-05-separating-modules-into-different-files.html#alternate-file-paths
- [Style Guide] https://doc.rust-lang.org/style-guide/ and https://doc.rust-lang.org/style-guide/items.html
- [API] Rust API Guidelines, naming: https://rust-lang.github.io/api-guidelines/naming.html
- [r-a] rust-analyzer style guide: https://github.com/rust-lang/rust-analyzer/blob/master/docs/book/src/contributing/style.md
- Clippy lint list and default limits: https://rust-lang.github.io/rust-clippy/master/index.html

Where these sources disagree, this file decides. Item order follows
rust-analyzer (`mod`, then `use`), not the Style Guide (`use`, then
`mod`), because the code already uses that order. ghit does not adopt
rust-analyzer's `anyhow` errors. Errors stay `String`s, because they end
up on the status line.

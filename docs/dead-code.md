# Dead code and deprecation — ghit guidelines

Delete aggressively; deprecate explicitly. Nothing in between.

## Sweep after every structural change

Any refactor, layer move, or backend swap orphans code. Run the
`dead-code-sweep` skill before calling the change done: compiler warnings
first, then per-symbol reference checks. The compiler only sees uncalled
functions — hunt write-only fields and unread fields too.

## Deprecate with a removal pointer

When old code must survive temporarily (like `gh.rs` until the GraphQL
board port), mark it so:

- `#[deprecated(note = "...")]` on the function, naming what replaces it.
- One shim at the single call site (`board_via_gh`), with an
  `#[allow(deprecated)]` that cites the removal plan — never a bare allow.
- A `DEPRECATED` header on the module: what moved, what remains, no new callers.
- Docs table updated (`AGENTS.md`, `docs/routers.md`).

## Delete when the replacement lands

A deprecation without a removal diff is just a comment. The follow-up removes
the shim, the module, and the allow — and the sweep confirms nothing else
referenced it.

## Gates

`cargo build` zero warnings, `cargo test` green, same as any behavior change.

# AGENTS.md — ghit contributor guide

`ghit` is a terminal viewer for GitHub issues and PRs, written in Rust,
backed by the `gh` CLI. Repo [`bolu-atx/gh-ghit`](https://github.com/bolu-atx/gh-ghit);
crate and binary are both called `ghit`.

## Commands

```bash
cargo build   # refreshes target/debug/ghit (also your PATH binary via symlink)
cargo test    # unit + headless frame tests, must be green with zero warnings
cargo build 2>&1 | grep -cE "^warning|^error"   # expect 0
```

Run the app with isolated state so pins/cache writes don't touch real data:

```bash
XDG_DATA_HOME=/tmp/ghit-e2e target/debug/ghit -R owner/repo 123
```

## Architecture

| File | Owns |
| ---- | ---- |
| `src/main.rs` | Thin entry: parse `cli`, run `app` |
| `src/cli.rs` | Clap `Args` + numeric startup parsing |
| `src/app.rs` | `App` state, event loop, background fetch wiring |
| `src/core.rs` | Pure business rules (`parse_bar`, selection, board cursor, pins, window) |
| `src/model.rs` | `IssueData`/`BoardData` types, pure helpers (`slug`, `linkify`, `refs_in_line`, `board_columns`) |
| `src/github.rs` | Data-access facade over `api`/`gh`/`cache`; UI layers never import those directly |
| `src/api.rs` | Token-backed GraphQL/REST calls |
| `src/cache.rs` | 30-min JSON cache + pinned-issue files under `$XDG_DATA_HOME` or `~/.local/share/ghit` |
| `src/gh.rs` | `gh` subprocess calls (issue/PR view, GraphQL project status, project board) |
| `src/input.rs` | Key/mouse mapping into `App` calls; reads `ui` hit areas, never draws |
| `src/ui.rs` | All rendering + frame tests; reads `App`, never fetches |
| `src/theme.rs` | Palettes (ayu mirage, ayu dark), `GhitSheet` markdown stylesheet, `cycle()` |

Key patterns:

- **Background fetch, instant UI.** The board modal opens with a skeleton;
  a thread fetches and the loop picks it up via `poll_board`. Do the same
  for any new network path — never block the event loop.
- **Capture the frame buffer inside `render`.** `terminal.current_buffer_mut()`
  after `draw` is the post-swap blank buffer. `click-to-open` refs depend on
  `body_rows` captured from the frame being drawn.
- **Theme through functions.** Never hardcode colors in render code; use
  `theme::ink()` etc. so `t` (toggle) repaints everything.
- **Stateful lists scroll themselves.** Board columns render statefully so the
  selected card stays visible; record click areas from `ListState::offset()`.

## Verification

- `cargo test` after every behavior change; add a frame test in `render_smoke`
  for UI changes (see `docs/routers.md` for the live-drive procedure).
- Live check in tmux before pushing anything interactive. Never press `o`
  during verification (it opens a browser).
- Mouse is real SGR: `tmux send-keys -l $'\x1b[<0;x;ym'` (press) and
  `$'\x1b[<3;x;my'` (release), 1-based cells.

## Commits and releases

- Title style: `feat: …`, `fix: …`, `docs: …`, `refactor: …`.
- The repo name must keep the `gh-` prefix and the root `gh-ghit` shim —
  that is what makes `gh extension install` work.

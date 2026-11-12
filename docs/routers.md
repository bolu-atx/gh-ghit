# Routers — where work goes in ghit

Small codebase, so route by layer, not by service. One change, one layer,
one verification pass.

## Route by layer

| Task | Layer | Verify with |
| ---- | ----- | ----------- |
| Fetching, caching, pins | `github.rs` over `api.rs` + `cache.rs` (`gh.rs` board-only, deprecated) | `cargo test` (unit) |
| Text transforms, ref parsing | `model.rs` | `cargo test` (unit) |
| Business rules (bar, selection, cursor, pins) | `core.rs` | `cargo test` (unit) |
| CLI args | `cli.rs` | `cargo test` (unit) |
| Theme, markdown stylesheet | `theme.rs` | `cargo test` + eyeball in tmux |
| Keys, mouse, modal flow | `input.rs` + `app.rs` | `cargo test` + live drive |
| Rendering, layout | `ui.rs` render fns | frame tests + live drive |

Cross-layer changes (e.g. a new data field shown in the UI) go data-first:
`model.rs` type + unit test, then `render_*` + frame test, then live drive.

## Review loop

1. `cargo build` — zero warnings or stop.
2. `cargo clippy --all-targets -- -D warnings` — zero lints or stop.
3. `cargo test` — full suite, zero failures.
4. Live drive for anything interactive (below). Fix what you see, then
   re-run from step 1.

## Live drive procedure

Isolated state, fixed geometry, scripted input over the drive socket —
no tmux, no TTY:

```bash
XDG_DATA_HOME=/tmp/ghit-e2e target/debug/ghit \
  --drive /tmp/ghit.sock -R owner/repo 123 &
mkfifo /tmp/ghit.cmd && exec {CMD}<>/tmp/ghit.cmd
python3 scripts/e2e/drive.py --sock /tmp/ghit.sock \
  --cmd /tmp/ghit.cmd --pane /tmp/ghit-pane.txt &
sleep 8   # first fetch
```

Drive one control per step and capture after each:

```bash
printf '%s\n' '{"type":"key","key":"j"}' >&$CMD
printf '%s\n' '{"type":"key","key":"2"}' >&$CMD
sed -n '1,12p' /tmp/ghit-pane.txt
```

Mouse is the same 1-based cells as SGR, press + release:

```bash
printf '%s\n' '{"type":"mouse","kind":"down-left","column":70,"row":25}' >&$CMD
printf '%s\n' '{"type":"mouse","kind":"up-left","column":70,"row":25}' >&$CMD
```

Rules: never press `o` (opens a browser). Refresh (`r`) and pins (`p`)
are safe under the isolated `XDG_DATA_HOME`. End every session with
`{"type":"quit"}`, then kill the app and remove the socket.

## Visual taste gate

Layout, theme, or new modal changes only.

Workers check structure with `frame_text` + the drive pane file.
That text strips style, so it cannot judge looks.

Coordinator judges looks: one PNG each of the loaded issue and the
board modal, at fixed geometry. Pass means one focus per frame,
aligned columns, no dead panes, theme colors only, and no
truncation at 200x50 and 120x40. Fail on crowding, weak contrast
in either palette, or repeated status info. Procedure:
[verification.md](verification.md).

## Coordinator owns e2e coverage

Each coordinated user-visible change adds one scenario:
scripted input plus expected pane rows. Visible means new or
changed controls, flows, or layout. Optimization, config,
refactor, and data-layer work with no visible change needs no
scenario. The coordinator picks it from the touched control,
records the exact drive-protocol lines sent plus the pane rows they
produce (`scripts/e2e/lib.sh` maps the old `send-keys` names to
protocol lines; assertions grep the latest frame file exactly like
they used to grep `capture-pane -p`), and requires the matching
`render_smoke` frame test for UI changes, using the template in
[verification.md](verification.md). No scenario for a visible
change, no sign-off.

## Architecture review

The coordinator runs one review at milestone close or every 5th
merged story PR, whichever comes first.

A read-only reviewer gets the diff range plus what the work was,
and judges structure only: layer table still true, core still pure,
third copies lifted, one-caller helpers inlined. Findings become a
`chore` issue on the board — ranked lift/change/delete items, or a
recorded no-change verdict. Procedure and template:
[verification.md](verification.md). The next milestone opens after
the issue is filed.

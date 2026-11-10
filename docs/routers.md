# Routers — where work goes in ghit

Small codebase, so route by layer, not by service. One change, one layer,
one verification pass.

## Route by layer

| Task | Layer | Verify with |
| ---- | ----- | ----------- |
| Fetching, caching, pins | `github.rs` facade (`api.rs`, `gh.rs`, `cache.rs`) | `cargo test` (unit) |
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
2. `cargo test` — full suite, zero failures.
3. Live drive for anything interactive (below). Fix what you see, then
   re-run from step 1.

## Live drive procedure

Isolated state, fixed geometry, scripted input:

```bash
tmux kill-session -t ghit-e2e 2>/dev/null
tmux new-session -d -s ghit-e2e -x 200 -y 50 \
  "XDG_DATA_HOME=/tmp/ghit-e2e target/debug/ghit -R owner/repo 123"
sleep 8   # first fetch
```

Drive one control per step and capture after each:

```bash
tmux send-keys -t ghit-e2e j k Down Up 1 2 PageDown PageUp b
tmux capture-pane -t ghit-e2e -p | sed -n '1,12p'
```

Mouse is real SGR (1-based cells, press + release):

```bash
tmux send-keys -t ghit-e2e -l $'\x1b[<0;70;25M'
tmux send-keys -t ghit-e2e -l $'\x1b[<3;70;25m'
```

Rules: never press `o` (opens a browser). Refresh (`r`) and pins (`p`)
are safe under the isolated `XDG_DATA_HOME`. End every session with `q`
and `tmux kill-session -t ghit-e2e`.

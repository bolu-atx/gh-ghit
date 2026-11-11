# Verification — taste gate and e2e scenarios

Companion to [routers.md](routers.md). Rules live there; commands
live here.

## Taste gate procedure

Coordinator only, on layout, theme, or new modal changes.

1. `cargo build` — zero warnings or stop.
2. Launch the live-drive session from routers.md (200x50).
3. Loaded issue: wait for fetch, take PNG #1.
4. Board modal: press `b`, take PNG #2.
5. Press `t` (toggle palette), repeat 3–4. Both palettes must pass.
6. Small screen: relaunch at 120x40, check truncation with
   `capture-pane -p`. Text suffices here; no PNG needed.

Screenshot the terminal window showing the session. Save as
`/tmp/ghit-taste-<state>-<palette>.png` and attach both to the PR.

Pass per PNG: one focus, aligned columns, no dead panes, theme
colors only, no truncation. Fail on crowding, weak contrast, or
repeated status info.

## E2e scenario template

Each coordinated user-visible change adds one scenario to the
PR body, plus the matching `render_smoke` frame test for UI
changes. Optimization, config, refactor, and data-layer work with
no visible change needs no scenario.

- Touched control:
- `send-keys`:
- `capture-pane`:
- Expected rows:
- Frame test:

One control per scenario. No scenario, no sign-off.

## Breaking existing tests

New features can break existing tests on purpose. When one fails,
judge which side is right: if the test encodes behavior the feature
meant to change, update the test; if the code regressed past the
feature's intent, fix the code. Record the call in the PR body —
one line per break: what failed, which side changed, why.

## Architecture review procedure

1. Coordinator gathers the diff range since the last review, plus a
   one-paragraph note per story on what the work was.
2. One read-only reviewer pass over that range: layer violations
   (`ui`/`input` ↔ `api`/`gh`/`cache`, `model`/`core` ↔
   ratatui/crossterm), duplication, dead code.
3. Reviewer judges each finding against the Principles in
   [code-style.md](code-style.md).
4. Coordinator files the `chore` issue below. No-change verdicts get
   filed too, with the rationale.

Issue template:

- Range reviewed:
- Lift (third copies):
- Change (abstractions that no longer fit):
- Delete (dead or one-caller code):
- Verdict: refactor issue(s) / no change, because:

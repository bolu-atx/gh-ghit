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

Each coordinated interactive change adds one scenario to the PR
body, plus the matching `render_smoke` frame test for UI changes.

- Touched control:
- `send-keys`:
- `capture-pane`:
- Expected rows:
- Frame test:

One control per scenario. No scenario, no sign-off.

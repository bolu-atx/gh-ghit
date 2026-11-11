# Planning — milestones, issues, PRs

We plan in GitHub issues and milestones, and we dogfood `ghit`
to read the plan (`ghit -R bolu-atx/gh-ghit` shows milestones,
sub-issues, blocked-by/blocking, and labels).

## Shape

- A **milestone is an epic**: one coherent story of work, 2–6 issues.
  Name it after the outcome (e.g. `PR review in the terminal`),
  not a version. One-line description states the done condition.
- An **issue is a coarse story**: one shippable behavior with
  its own what, how, and verification. If an issue needs sub-tasks
  to track, it is probably two stories — split it instead.
  Sub-issues are for genuine decomposition only, not for chapters
  of one story.
- **Relationships** record real dependencies between issues:
  `blocked-by` / `blocking` when order matters, parent/sub-issue
  when one story truly contains another. Do not link related work
  that can land in any order — mention it in `How` instead.
- **Board**: every issue lives on the
  [`ghit dev` board](https://github.com/users/bolu-atx/projects/1).
  Create with `gh issue create -p 'ghit dev' …` or add right after
  filing — no issue floats outside the board.

## Issue body

Plain technical English. Terse. Four sections, in this order:

```md
## What & why

One paragraph: what changes for the user, and why it matters.

## How

Steps and phases to get there, in order. Name upstream
dependencies (issues, PRs, external work) explicitly.

## Verification performed

Commands run and what was observed (tests, live drive, logs).
Quote results; do not claim what was not run.

## Agents supplemental

Prompt (or pointer to it) and verifier used, so the next agent
can reproduce the work.
```

One idea per sentence. No filler, no headers beyond these four.

## Labels

Labels say what an issue is **about**, not its status or priority:

| Label     | About                              |
| --------- | ---------------------------------- |
| `feature` | New behavior or request            |
| `bugfix`  | Something is not working           |
| `docs`    | Documentation, demo, release notes |
| `chore`   | Build, CI, release, maintenance    |

One subject label per issue. Workflow labels (`duplicate`,
`invalid`, `wontfix`, …) stay as-is.

## Pull requests

- Every PR solves an issue. The description opens with
  `Closes #N` (or `Fixes #N`) so merge auto-closes the issue.
- No PR without an issue. If none fits, file the story first —
  coarse is fine, one paragraph per section to start.
- Branch and title follow [AGENTS.md](AGENTS.md): `feat/…`,
  `fix/…`, `docs/…`, `refactor/…`, `chore/…`.

## Agent guidelines

- Start from the plan: open `ghit`, pick the milestone, read the
  story and its `How` before touching code.
- Keep the story current while you work: check off phases in
  `How`, append real `Verification performed` output, record your
  prompt and verifier under `Agents supplemental`.
- File one issue per story you discover, straight onto the board
  (`-p 'ghit dev'`); link ordering with `blocked-by` only when
  landing order truly matters.
- Track status on the board: mark the story `In Progress` when work
  starts. When the PR opens, add the PR to the board as well (`gh pr
  create` takes no `-p` flag, so follow with `gh project item-add 1
  --owner bolu-atx --url <pr-url>`) and move both to `In Review`.
- Never press `o` during verification (it opens a browser).
  End every tmux live drive with `q` and kill the session.

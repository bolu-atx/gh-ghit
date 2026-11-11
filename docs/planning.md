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
  filing — no issue floats outside the board. Status moves and
  the working loop live in [CONTRIBUTING.md](../CONTRIBUTING.md).

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
- Open is not done. A PR counts as done only when mergeable:
  `gh pr checks` green, no merge conflicts with `main`,
  review comments addressed. Re-check after every push —
  a green run on an old head says nothing about the new one.
- Merge promptly once mergeable. A mergeable PR left open
  rots: rebase onto `main` and re-verify if it falls behind.

## Working the plan

Doing the work — board moves, coordinator dispatch, merge bar —
lives in [CONTRIBUTING.md](../CONTRIBUTING.md). Agents read that
first.

# Contributing — agent workflow

You are doing the work. Plan conventions live in
[docs/planning.md](docs/planning.md); this is the doing.

## Start from the plan

Open `ghit`, pick the milestone, read the story and its `How`
before touching code.

## While you work

- Workers implement the story's `How`, run verification, and
  report back. They never edit the story body or touch the board.
- The coordinator owns all tracking: check off phases in `How`,
  append real `Verification performed` output, record prompts and
  verifiers under `Agents supplemental`, move the cards.
- One issue per story discovered: the worker reports it, the
  coordinator files it onto the board (`-p 'ghit dev'`);
  `blocked-by` only when landing order truly matters.
- Single session: the coordinator is also the worker — update the
  story as you go.

## Board status

- Mark the story `In Progress` when work starts. When the PR
  opens, add the PR to the board as well (`gh pr create` takes
  no `-p` flag, so follow with `gh project item-add 1
  --owner bolu-atx --url <pr-url>`) and move both to `In Review`.
- Coordinator owns the cards end to end and is the only one
  who moves them. State the issue number and the board commands
  in each worker prompt; a fresh worker never knows the board.
  Single session: move the cards yourself at the two moments
  above.
- Only the merge moves cards to Done. Never call a story done on
  PR open: the worker's last step is to prove mergeability
  (`gh pr checks` green, no conflicts), the coordinator merges.

## Safety

- Never press `o` during verification (it opens a browser).
  End every tmux live drive with `q` and kill the session.

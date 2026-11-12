# Releasing — agent procedure

End-to-end flow for shipping a release. The agent drives; the human
confirms scope and version, then the agent runs everything.

## 1. Confirm scope and version

- `git log <prev-tag>..main --oneline` is the candidate set.
- Ask the human: what goes in, and patch or minor. Feats since the
  last tag mean minor. Confirm before touching anything.

## 2. Draft the UX summary

Read the commits going out and write 3–8 short lines in
user-experience language: what the user sees and why it matters.
No file names, no architecture, no commit hashes. This text becomes
the annotated tag message and the top of the release notes, above
the auto-generated changelog. Example shape:

```text
Board milestone pills: cards now show their milestone at a glance,
and the board header links straight to the milestones page.
Review submit: pending comments can be submitted as one review
from the compose pane instead of one by one.
```

## 3. Run the driver

Save the summary to a file (e.g. `/tmp/release-notes.md`) and run:

```bash
scripts/release.sh <version> --tldr-file /tmp/release-notes.md
```

Without `--yes` it only prints the plan. Re-run with `--yes` after
the human approved the plan. The script then:

1. Opens a `chore/bump-X` PR (`Cargo.toml` + `Cargo.lock`),
   waits for checks, squash-merges.
2. Creates annotated tag `vX` with the summary as its message and
   pushes it. The push fires the [release workflow](../.github/workflows/release.yml),
   which fails fast if the tag and `Cargo.toml` disagree.
3. Watches the workflow and prints the release URL plus assets.

Manual fallback: `gh workflow run release --ref main -f tag=vX.Y.Z
-f tldr="..."` re-runs publishing for an existing tag, with the
`tldr` input overriding the tag message.

## 4. Verify and report

The script already watches the run to green and lists the attached
binaries. Report the release URL. If the run fails, the tag stays
and binaries stay unpublished — fix forward on `main`, never by
moving the tag.

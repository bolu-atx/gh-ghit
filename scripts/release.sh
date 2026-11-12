#!/usr/bin/env bash
# scripts/release.sh — agent-driven release driver for ghit.
#
# Usage:
#   scripts/release.sh <version> (--tldr "..." | --tldr-file <file>) [--yes]
#
# What it does:
#   1. Preflight: clean tree, gh authed, version newer than Cargo.toml.
#   2. Bump PR: chore/bump-X branch with Cargo.toml + Cargo.lock,
#      cargo test, push, open PR, wait for checks, squash-merge.
#   3. Tag: annotated tag vX whose message is the UX summary, push it.
#      Pushing the tag fires the release workflow, which puts the tag
#      message on top of the auto-generated changelog.
#   4. Watch the release run and verify the release + binaries.
#
# The UX summary (the --tldr text) is written by the calling agent, in
# human-readable user-experience language, before invoking this script.
# See docs/releasing.md for the full agent procedure.
set -euo pipefail

die() { echo "release.sh: $*" >&2; exit 1; }

VERSION=""
TLDR=""
TLDR_FILE=""
YES=0

while [ $# -gt 0 ]; do
  case "$1" in
    --tldr) TLDR="${2:-}"; shift 2 ;;
    --tldr-file) TLDR_FILE="${2:-}"; shift 2 ;;
    --yes) YES=1; shift ;;
    -*) die "unknown flag $1" ;;
    *) [ -z "$VERSION" ] || die "unexpected arg $1"; VERSION="$1"; shift ;;
  esac
done

[ -n "$VERSION" ] || die "usage: release.sh <version> (--tldr ... | --tldr-file <file>) [--yes]"
VERSION="${VERSION#v}"
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]] || die "bad version '$VERSION', want X.Y.Z"
TAG="v$VERSION"

if [ -n "$TLDR_FILE" ]; then
  [ -f "$TLDR_FILE" ] || die "tldr file not found: $TLDR_FILE"
  TLDR="$(cat "$TLDR_FILE")"
fi
[ -n "$TLDR" ] || echo "warning: empty TL;DR, release notes will be generated changelog only"

# --- Preflight ---
git diff --quiet && git diff --cached --quiet || die "working tree not clean"
command -v gh >/dev/null || die "gh CLI not found"
gh auth status >/dev/null 2>&1 || die "gh not authenticated"
[ "$(git branch --show-current)" = "main" ] || die "must run from main"
git fetch origin --tags --quiet
git status -sb | head -1 | grep -q 'behind' && die "main is behind origin, pull first"
CURRENT="$(grep '^version' Cargo.toml | head -1 | sed -E 's/.*"([^"]+)".*/\1/')"
if [ "$CURRENT" = "$VERSION" ]; then die "Cargo.toml already at $VERSION"; fi
if [ "$(printf '%s\n%s\n' "$CURRENT" "$VERSION" | sort -V | tail -1)" != "$VERSION" ]; then
  die "version $VERSION is not newer than Cargo.toml $CURRENT"
fi
PREV_TAG="$(git tag --sort=-v:refname | grep -E '^v[0-9]' | head -1 || true)"
[ -n "$PREV_TAG" ] || die "no previous v* tag found"
git rev-parse "$TAG" >/dev/null 2>&1 && die "tag $TAG already exists"
COMMITS="$(git log --oneline "$PREV_TAG..HEAD")"
[ -n "$COMMITS" ] || die "nothing new since $PREV_TAG"

echo "=== release plan ==="
echo "  $PREV_TAG (Cargo $CURRENT) -> $TAG"
echo "  commits going out:"
echo "$COMMITS" | sed 's/^/    /'
echo "  TL;DR:"
echo "$TLDR" | sed 's/^/    /'
if [ "$YES" -ne 1 ]; then
  echo "re-run with --yes to execute"
  exit 0
fi

# --- Bump PR ---
BRANCH="chore/bump-$VERSION"
git switch -c "$BRANCH" --quiet
sed -i.bak -E "s/^version = \".*\"/version = \"$VERSION\"/" Cargo.toml
rm -f Cargo.toml.bak
cargo test 2>&1 | tail -2
git add Cargo.toml Cargo.lock
git commit --quiet -m "chore: bump to $VERSION for release"
git push --quiet -u origin "$BRANCH"
PR_URL="$(gh pr create --base main --title "chore: bump to $VERSION for release" \
  --body "Bump Cargo.toml and the locked entry $CURRENT -> $VERSION for release. Tag \`$TAG\` on main after merge.")"
PR_NUM="${PR_URL##*/}"
echo "PR #$PR_NUM: $PR_URL"
gh pr checks "$PR_NUM" --watch --fail-fast
gh pr merge "$PR_NUM" --squash --delete-branch
git switch --quiet main
git pull --ff-only --quiet origin main
git branch -D "$BRANCH" 2>/dev/null || true
[ "$(grep '^version' Cargo.toml | head -1 | sed -E 's/.*"([^"]+)".*/\1/')" = "$VERSION" ] \
  || die "main is not at $VERSION after merge, aborting before tag"

# --- Tag + release ---
git tag -a "$TAG" -m "$TLDR"
git push origin "$TAG"
echo "tag $TAG pushed, release workflow started"

# --- Watch + verify ---
RUN_ID=""
for _ in $(seq 1 30); do
  RUN_ID="$(gh run list --workflow release --limit 10 \
    --json databaseId,headBranch --jq "[.[] | select(.headBranch==\"$TAG\")][0].databaseId // empty")"
  [ -n "$RUN_ID" ] && break
  sleep 10
done
[ -n "$RUN_ID" ] || die "release run for $TAG not found; check gh run list"
gh run watch "$RUN_ID"
gh release view "$TAG" --json tagName,name,assets --jq \
  '{tag: .tagName, assets: [.assets[].name]}'
echo "done: $(gh release view "$TAG" --json url --jq .url)"

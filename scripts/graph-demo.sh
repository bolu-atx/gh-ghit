#!/bin/bash
# Try the dependency map by hand on a dense synthetic repo, fully offline.
#
# Builds ghit, writes ~80 demo items with blocking chains, milestones,
# PRs and orphans into a throwaway store (store/demo_tests.rs), then opens
# `demo/graph` at #24; press 3 for the Map tab. Every network call goes to a
# closed local port and fails fast, so nothing touches GitHub or your
# real data dir. Extra arguments go to ghit (for example `--drive SOCK`).
#
#   scripts/graph-demo.sh          # opens #24
#   ISSUE=3 scripts/graph-demo.sh  # opens #3
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
unset GH_HOST
data="$(mktemp -d "${TMPDIR:-/tmp}/ghit-graph-demo.XXXXXX")"
trap 'rm -rf "$data"' EXIT
export XDG_DATA_HOME="$data"
export GH_TOKEN=demo-fake
export GHIT_API_BASE=http://127.0.0.1:9
export GHIT_NO_BACKFILL=1

cargo build --quiet
GHIT_DEMO_STORE="$data/ghit/store/github.com/demo/graph.sqlite" \
  cargo test --quiet -p ghit-core --lib store::demo_tests::write_graph_demo_store -- --ignored >/dev/null
./target/debug/ghit -R demo/graph "$@" "${ISSUE:-24}"

#!/bin/bash
# Run every e2e scenario in its own tmux session. Exit 0 only when all pass.
#
# Fixture contract (override with E2E_REPO / E2E_NUMS): the repo must have
# an issue numbered like 9, PRs numbered like 19 and 24 where 24's body
# mentions another ticket and 24 carries a multi-file diff. Numbers and
# types are immutable, but a body edit that drops the ref breaks
# scenario-refs-yank — read a red build as fixture rot first.
set -u
cd "$(dirname "$0")"
export E2E_FRAMES="${E2E_FRAMES:-/tmp/ghit-e2e-frames}"
export E2E_TIMEOUT="${E2E_TIMEOUT:-180}"
rm -rf "$E2E_FRAMES"; mkdir -p "$E2E_FRAMES"
pass=0; failed=0; failed_names=""
for s in scenario-*.sh; do
  export E2E_SCENARIO="$s"
  echo "=== $s"
  if timeout "$E2E_TIMEOUT" bash "$s" > "$E2E_FRAMES/$s.log" 2>&1; then
    echo "--- PASS $s"; pass=$((pass + 1))
  else
    echo "--- FAIL $s (see $E2E_FRAMES/$s.log)"; tail -4 "$E2E_FRAMES/$s.log"
    failed=$((failed + 1)); failed_names="$failed_names $s"
  fi
done
echo "scenarios: $pass passed, $failed failed$failed_names"
[ "$failed" -eq 0 ]

#!/bin/bash
# Fixture: startup opens 9, 19, 24. The mock answers too fast to catch
# the loading body reliably, so this asserts the settled state: once the
# fetches land, no pane keeps a loading body or a title spinner.
source "$(dirname "$0")/lib.sh"
e2e_start
e2e_wait "/pull/24" 20 || fail "startup issue never loaded"
e2e_wait "Issues" 20 || fail "sidebar never rendered"
for _ in $(seq 1 20); do
  e2e_pane | grep -qE "Loading[^ ]* ?(#[0-9]+|issues|pull requests|board)?…" || break
  sleep 1
done
e2e_shot pane-loading-settled
e2e_pane | grep -qE "Loading[^ ]* ?(#[0-9]+|issues|pull requests|board)?…" && fail "loading body still showing after load"
e2e_pane | grep -E "Issues|Pull requests|Refs" | grep -q "[⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏]" && fail "pane title spinner still showing after load"
e2e_pane | grep -q "No issue loaded" && fail "main pane never filled"
echo "PASS pane loading settles"

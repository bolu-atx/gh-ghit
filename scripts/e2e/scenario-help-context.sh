#!/bin/bash
# Fixture: default. The help screen is generated from the keymap and lists
# the context it was opened from first, marked "▸ ... · here". Open it from
# the Issues pane, then from the PRs pane, and check the first section.
source "$(dirname "$0")/lib.sh"
e2e_start
e2e_wait "move title" 60 || fail "pr data never landed"
# Title of the first marked section in the drawn help.
first_section() {
  e2e_pane | grep -o '▸ [^│]* · here' | head -1 | sed 's/ *$//'
}
expect_first() { # $1 title
  local end=$(( $(date +%s) + 10 )) got=""
  while [ "$(date +%s)" -lt "$end" ]; do
    got="$(first_section)"; [ "$got" = "$1" ] && return 0; sleep 0.5
  done
  fail "expected first section '$1', got '$got'"
}
e2e_key '?'
e2e_wait "Shortcuts" 10 || fail "help did not open"
expect_first "▸ Issues pane · here"
e2e_pane | grep -q "Evict row" || fail "issues keys missing from the first page"
e2e_shot help-issues
e2e_key Escape
e2e_key Tab
e2e_key '?'
expect_first "▸ Pull requests pane · here"
e2e_shot help-prs
e2e_key Escape
echo "PASS help context"

#!/bin/bash
# Fixture: PR 24's body mentions another ticket (Refs pane) and bar jumps
# fill History. Tab walks every visible pane in screen order, Main included;
# BackTab reverses. Only the focused pane draws a double top border (╔…╗),
# so the title inside it names the focus. Main has no box: its tab rule
# turns double (═) while no top border is double.
source "$(dirname "$0")/lib.sh"
e2e_start
e2e_wait "move title" 60 || fail "pr data never landed"
e2e_bar 9
e2e_wait "/issues/9" 20 || fail "show #9"
e2e_bar 24
e2e_wait "/pull/24" 20 || fail "show #24"
e2e_wait "History" 20 || fail "history pane missing"
e2e_wait "Refs" 20 || fail "refs pane missing"
# Name of the pane whose top border is double, or Main when none is.
focus() {
  local top; top="$(e2e_pane | grep -o '╔[^╗]*╗' | head -1)"
  if [ -z "$top" ]; then
    e2e_pane | grep -q '═══' && echo Main || echo none
    return
  fi
  for name in Issues "Pull requests" Refs Filters History; do
    case "$top" in *"$name"*) echo "$name"; return ;; esac
  done
  echo "?"
}
expect_focus() { # $1 name
  local end=$(( $(date +%s) + 10 )) got=""
  while [ "$(date +%s)" -lt "$end" ]; do
    got="$(focus)"; [ "$got" = "$1" ] && return 0; sleep 0.5
  done
  fail "expected focus on $1, got $got"
}
expect_focus Issues
e2e_shot focus-issues
e2e_key Tab; expect_focus "Pull requests"
e2e_key Tab; expect_focus Main
e2e_shot focus-main
e2e_key Tab; expect_focus Refs
e2e_key Tab
# Filters sits between Refs and History when the repo has labels.
[ "$(focus)" = Filters ] && e2e_key Tab
expect_focus History
e2e_shot focus-history
e2e_key Tab; expect_focus Issues
e2e_key BTab; expect_focus History
e2e_key BTab
[ "$(focus)" = Filters ] && e2e_key BTab
expect_focus Refs
echo "PASS focus cycle"

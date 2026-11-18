#!/bin/bash
# Fixture: startup opens 9, 19, 24. The Main tab line carries a toolbar.
# A click on its close button asks first; confirming sends one PATCH to
# the mock, which flips the issue, and the toolbar then reads reopen.
# The keyboard path (C) reopens it. Mouse cells are 1-based like SGR; the
# pane scan is 0-based.
source "$(dirname "$0")/lib.sh"
mock_log="$E2E_FRAMES/$E2E_SCENARIO.log"
e2e_start
e2e_bar 9
e2e_wait "/issues/9" 20 || fail "show #9"
e2e_wait "Issues 1" 20 || fail "sidebar never listed #9"
e2e_wait "comment" 20 || fail "toolbar missing comment"

# 1-based row and column of a word on the tab line.
tool_cell() { # $1 word
  e2e_pane | python3 -c '
import sys
word = sys.argv[1]
for i, line in enumerate(sys.stdin.read().splitlines()):
    if "1 Details" in line and word in line:
        print(i + 1, line.index(word) + 1)
        break
' "$1"
}
click() { # $1 row, $2 col
  e2e_key -l "$(printf '\033[<0;%s;%sM' "$2" "$1")"
  e2e_key -l "$(printf '\033[<3;%s;%sm' "$2" "$1")"
}

read -r row col < <(tool_cell close)
[ -n "${row:-}" ] || fail "close button not found on the tab line"
e2e_shot toolbar-open
click "$row" "$col"
e2e_wait "Close #9\?" 20 || fail "close click never asked"
grep -q "mock PATCH" "$mock_log" && fail "PATCH sent before the confirm"
e2e_shot toolbar-confirm
# Cancel first: nothing is written.
e2e_key Escape
e2e_wait "reopen|close" 5 || true
e2e_pane | grep -q "Close #9?" && fail "Esc left the dialog open"
grep -q "mock PATCH" "$mock_log" && fail "cancel sent a PATCH"

read -r row col < <(tool_cell close)
click "$row" "$col"
e2e_wait "Close #9\?" 20 || fail "second close click never asked"
e2e_key c
e2e_wait "closed #9" 20 || fail "confirm never closed #9"
grep -q 'mock PATCH /repos/bolu-atx/gh-ghit/issues/9 {"state": "closed"}' "$mock_log" || fail "no closed PATCH recorded"
e2e_wait "reopen" 20 || fail "toolbar never flipped to reopen"
e2e_shot toolbar-closed

# Keyboard path: C asks, r confirms the reopen.
e2e_key C
e2e_wait "Reopen #9\?" 20 || fail "C never asked to reopen"
e2e_key r
e2e_wait "reopened #9" 20 || fail "confirm never reopened #9"
grep -q 'mock PATCH /repos/bolu-atx/gh-ghit/issues/9 {"state": "open"}' "$mock_log" || fail "no open PATCH recorded"
e2e_wait "close" 20 || fail "toolbar never flipped back to close"
e2e_shot toolbar-reopened
echo "PASS toolbar closes and reopens with a confirm"

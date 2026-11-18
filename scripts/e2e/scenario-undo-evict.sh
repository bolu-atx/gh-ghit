#!/bin/bash
# Fixture: startup opens 9, 19, 24. `x` evicts the highlighted sidebar row
# and arms an undo toast; `u` inside the window brings the row back and
# re-selects it. Keyboard only (mouse cells are not 1->0 converted, #173).
source "$(dirname "$0")/lib.sh"
e2e_start
e2e_bar 9
e2e_wait "/issues/9" 20 || fail "show #9"
e2e_wait "Issues 1" 20 || fail "sidebar never listed #9"
e2e_key x
e2e_wait "removed #9 · u undo" 10 || fail "evict toast missing"
e2e_shot undo-evicted
e2e_pane | grep -q "Issues 1" && fail "row survived the evict"
e2e_key u
e2e_wait "restored #9" 10 || fail "undo toast missing"
e2e_wait "/issues/9" 10 || fail "undo never re-selected #9"
e2e_wait "Issues 1" 10 || fail "row not back in the sidebar"
e2e_shot undo-restored
echo "PASS undo evict"

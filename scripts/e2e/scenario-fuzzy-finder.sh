#!/bin/bash
# Fixture: startup opens 9, 19, 24. Ctrl+P opens the finder; typing part of
# a title lists the row; Enter opens that item in Main and closes the finder.
source "$(dirname "$0")/lib.sh"
e2e_start
e2e_bar 9
e2e_wait "/issues/9" 20 || fail "show #9"
e2e_key C-p
e2e_wait "Go to issue or PR" 20 || fail "ctrl-p never opened the finder"
e2e_shot finder-empty
e2e_type "unif"
e2e_wait "1 of [0-9]+" 20 || fail "query never narrowed the results"
e2e_pane | grep -q "#19  *unified interface" || fail "matching row missing"
e2e_shot finder-query
e2e_key Enter
e2e_wait "/pull/19" 20 || fail "enter never opened #19 in Main"
e2e_pane | grep -q "Go to issue or PR" && fail "finder stayed open after Enter"
e2e_shot finder-opened
echo "PASS ctrl-p finds and opens an item"

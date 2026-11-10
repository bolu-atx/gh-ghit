#!/bin/bash
# Fixture: issue 9 exists; its sidebar row renders once fetched.
source "$(dirname "$0")/lib.sh"
e2e_start
e2e_wait "Precompiled" 60 || fail "issue data never landed"
e2e_bar 9
e2e_wait "■ #9 " 20 || fail "show #9"
[ "$(e2e_markers)" -eq 1 ] || fail "marker count $(e2e_markers)"
e2e_key y
e2e_shot issue-yank
[ -n "$(e2e_toast)" ] || fail "no yank toast"
echo "toast: $(e2e_toast)"
echo "PASS issue yank"

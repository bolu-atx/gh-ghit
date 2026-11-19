#!/bin/bash
# Fixture: PR 24 has a multi-file diff. The status bar shows the focused
# context's keys from the registry: Issues keys first, Diff keys once Main
# has focus on the Diff tab. Keyboard only.
source "$(dirname "$0")/lib.sh"
e2e_start
e2e_wait "move title" 60 || fail "pr data never landed"
e2e_wait "x evict row" 10 || fail "issues hints missing: $(e2e_pane | tail -2)"
e2e_pane | grep -q "J next file" && fail "diff hints show in Issues"
e2e_shot hints-issues
e2e_bar 24
e2e_wait "/pull/24" 20 || fail "show #24"
e2e_key 2
e2e_wait "files ·" 60 || fail "diff summary never rendered"
# Diff keys act only while Main has focus.
e2e_key Tab Tab
e2e_wait '═══' 10 || fail "Main never took focus"
e2e_wait "J next file" 10 || fail "diff hints missing: $(e2e_pane | tail -2)"
e2e_shot hints-diff
echo "PASS bar hints follow context"

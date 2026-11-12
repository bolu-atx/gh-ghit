#!/bin/bash
# Fixture: PR 24's body mentions another ticket, so the Refs pane appears.
source "$(dirname "$0")/lib.sh"
e2e_start
e2e_wait "move title" 60 || fail "pr data never landed"
e2e_bar 24
e2e_wait "/pull/24" 20 || fail "show #24"
e2e_key Tab
e2e_key Tab
e2e_shot refs-focus
[ "$(e2e_markers)" -le 1 ] || fail "marker leak $(e2e_markers)"
e2e_key j
e2e_wait "█" 10 || fail "highlight never landed"
e2e_shot refs-highlight
# Sidebar keeps the current-issue marker while refs focus holds the
# highlight: one for the active row, one for the highlighted ref.
[ "$(e2e_markers)" -eq 2 ] || fail "refs marker count $(e2e_markers)"
e2e_key y
e2e_shot refs-yank
e2e_toast | grep -q "refs #" || fail "refs yank toast: $(e2e_toast)"
echo "toast: $(e2e_toast)"
echo "PASS refs yank"

#!/bin/bash
# Fixture: PR 24 has a multi-file diff.
source "$(dirname "$0")/lib.sh"
e2e_start
e2e_wait "move title" 60 || fail "pr data never landed"
e2e_bar 24
e2e_wait "/pull/24" 20 || fail "show #24"
e2e_key 2
e2e_wait "files ·" 60 || fail "diff summary never rendered"
e2e_shot diff-folded
e2e_key Space
e2e_wait "@@" 20 || fail "expand never revealed hunks"
e2e_shot diff-open
e2e_key Down
e2e_key Down
e2e_shot diff-line
e2e_key y
e2e_wait 'yanked #24.*"' 10 || fail "diff line toast: $(e2e_toast)"
e2e_shot diff-yank
echo "toast: $(e2e_toast)"
echo "PASS diff line yank"

#!/bin/bash
# Fixture: PR 24 has two small files and one over-threshold file (src/big.rs).
source "$(dirname "$0")/lib.sh"
e2e_start
e2e_wait "move title" 60 || fail "pr data never landed"
e2e_bar 24
e2e_wait "/pull/24" 20 || fail "show #24"
e2e_key 2
e2e_wait "files ·" 60 || fail "diff summary never rendered"
# Diff keys act only while Main has focus.
e2e_key Tab Tab
e2e_wait '═══' 10 || fail "Main never took focus"
e2e_pane | grep -q "new line" && fail "files should start folded"
e2e_key a
e2e_wait "new line" 20 || fail "expand-all never revealed the small file"
e2e_wait "large diff" 20 || fail "large file header note missing"
e2e_pane | grep -q "bigline0" && fail "large file expanded"
e2e_shot diff-expanded
e2e_key a
e2e_wait "folded all files" 10 || fail "second press did not fold"
e2e_pane | grep -q "new line" && fail "fold-all left a file open"
e2e_shot diff-refolded
echo "PASS diff expand all"

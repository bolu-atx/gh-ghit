#!/bin/bash
# Fixture: startup opens 9, 19, 24; the sidebar lists recent issues and PRs.
# Key grammar: `g` waits in the bar and `gg` / `G` jump to the first and last
# row of the focused list, `Enter` opens the selected row, `s` cycles the
# sort, `q` closes the help dialog instead of quitting, `Esc` at the base
# does nothing. Keyboard only.
source "$(dirname "$0")/lib.sh"
e2e_start
e2e_wait "move title" 60 || fail "pr data never landed"
e2e_bar 9
e2e_wait "/issues/9" 20 || fail "show #9"
e2e_shot grammar-start
e2e_key s
e2e_wait "updated" 10 || fail "s never cycled the sort"
e2e_key g
e2e_wait "g…" 10 || fail "pending g missing from the bar"
e2e_key g
e2e_pane | grep -q "g…" && fail "gg left the prefix showing"
e2e_shot grammar-gg
e2e_key G
e2e_shot grammar-G
e2e_key Enter
e2e_shot grammar-enter
e2e_key '?'
e2e_wait "Shortcuts" 10 || fail "help never opened"
e2e_key q
e2e_pane | grep -q "Shortcuts" && fail "q did not close help"
e2e_pane | grep -q "ghit" || fail "q quit instead of closing help"
e2e_key Escape
e2e_pane | grep -q "ghit" || fail "Esc at the base quit"
echo "PASS key grammar"

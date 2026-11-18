#!/bin/bash
# Fixture: PR #24 closes #9, so the right column (Refs, Filters) is wanted.
# At 90 columns the sidebar (44) leaves main 46, so the right column (36)
# must hide. The PR title "move title into header" stays on one row in
# main, and no Refs or Filters box draws. Tab must skip the hidden panes:
# Issues, Pull requests, Main, back to Issues.
# fixtures: labels
E2E_GEO=90x30
source "$(dirname "$0")/lib.sh"
e2e_start
e2e_wait "move title" 60 || fail "pr data never landed"
e2e_bar 24
e2e_wait "PR · OPEN" 20 || fail "show #24"
e2e_shot narrow-main
e2e_pane | grep -q '║#24 move title into header' || fail "title wrapped or missing in main"
e2e_pane | grep -qE "Refs|Filters" && fail "right column drew at 90 columns"
e2e_pane | grep -q "Issues" || fail "sidebar missing at 90 columns"
e2e_key Tab Tab Tab
e2e_pane | grep -o '╔[^╗]*╗' | head -1 | grep -q "Issues" || fail "Tab did not skip the hidden panes"
echo "PASS narrow main"

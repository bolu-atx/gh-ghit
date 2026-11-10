#!/bin/bash
# Fixture: PRs 19 and 24 exist; the first PR row selects #19.
# Covers mouse selection plus the at-most-one-marker focus discipline.
source "$(dirname "$0")/lib.sh"
e2e_start
e2e_wait "unifie" 60 || fail "pr rows never landed"
e2e_wait "move title" 60 || fail "pr data never landed"
e2e_wait "─ (requested|recent) ─" 60 || fail "labeled divider never rendered"
e2e_shot mouse-boot
R=$(grep -n "Pull requests" "$E2E_FRAMES/mouse-boot.txt" | head -1 | cut -d: -f1)
[ -n "$R" ] || fail "no pr pane row"
CLICKROW=$((R + 2))
e2e_key -l "$(printf '\x1b[<0;4;%dM' "$CLICKROW")"
e2e_key -l "$(printf '\x1b[<3;4;%dm' "$CLICKROW")"
e2e_wait "gh-ghit  #19" 20 || fail "mouse never selected #19"
e2e_shot mouse-select
e2e_key Tab
[ "$(e2e_markers)" -le 1 ] || fail "marker leak after Tab"
e2e_key Tab
[ "$(e2e_markers)" -le 1 ] || fail "marker leak after Tab Tab"
echo "PASS mouse select and focus travel"

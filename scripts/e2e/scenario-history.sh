#!/bin/bash
# Fixture: startup opens 9, 19, 24. Bar jumps record history, so Ctrl+O
# walks back and Ctrl+I redoes. Plain C-i arrives as Tab (no modifiers),
# so forward goes through the kitty CSI-u sequence for Ctrl+I instead.
source "$(dirname "$0")/lib.sh"
e2e_start
e2e_bar 9
e2e_wait "/issues/9" 20 || fail "show #9"
e2e_bar 24
e2e_wait "/pull/24" 20 || fail "show #24"
# Back to 9, then redo to 24.
e2e_key C-o
e2e_wait "/issues/9" 20 || fail "ctrl-o never returned to #9"
e2e_shot history-back
e2e_key Escape '[105;5u'
e2e_wait "/pull/24" 20 || fail "ctrl-i never redid #24"
e2e_shot history-forward
# A history click skips back to 9; Ctrl+I replays the skipped issue.
e2e_bar 19
e2e_wait "/pull/19" 20 || fail "show #19"
e2e_wait "History" 20 || fail "history panel missing"
row=$(e2e_pane | python3 -c 'import sys; lines=sys.stdin.read().splitlines(); start=next(i for i, line in enumerate(lines) if "History" in line[-36:]); print(next((i for i, line in enumerate(lines) if i > start and "#9 " in line[-36:]), 0))')
[ "$row" -gt 0 ] || fail "history #9 row missing"
cols=${E2E_GEO%x*}
e2e_key -l "$(printf '\033[<0;%s;%sM' "$((cols - 34))" "$row")"
e2e_wait "/issues/9" 20 || fail "history click never returned to #9"
e2e_key Escape '[105;5u'
e2e_wait "/pull/24" 20 || fail "ctrl-i never replayed #24 after click"
e2e_key Escape '[105;5u'
e2e_wait "/pull/19" 20 || fail "ctrl-i never replayed #19 after click"
# Back and redo with the keyboard.
e2e_key C-o
e2e_wait "/pull/24" 20 || fail "ctrl-o never returned to #24"
e2e_key C-o
e2e_wait "/issues/9" 20 || fail "ctrl-o never returned to #9"
e2e_key Escape '[105;5u'
e2e_wait "/pull/24" 20 || fail "ctrl-i never redid #24"
e2e_key Escape '[105;5u'
e2e_wait "/pull/19" 20 || fail "ctrl-i never redid #19"
e2e_shot history-redo
echo "PASS issue history"

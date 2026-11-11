#!/bin/bash
# Fixture: startup opens 9, 19, 24. Bar jumps record history, so Ctrl+O
# walks back and Ctrl+I redoes. Plain C-i arrives as Tab (no modifiers),
# so forward goes through the kitty CSI-u sequence for Ctrl+I instead.
source "$(dirname "$0")/lib.sh"
e2e_start
e2e_bar 9
e2e_wait "■ #9 " 20 || fail "show #9"
e2e_bar 24
e2e_wait "■ #24 " 20 || fail "show #24"
# Back to 9, then redo to 24.
e2e_key C-o
e2e_wait "■ #9 " 20 || fail "ctrl-o never returned to #9"
e2e_shot history-back
e2e_key Escape '[105;5u'
e2e_wait "■ #24 " 20 || fail "ctrl-i never redid #24"
e2e_shot history-forward
# A new jump clears forward: 19, back, back, forward, forward.
e2e_bar 19
e2e_wait "■ #19 " 20 || fail "show #19"
e2e_key C-o
e2e_wait "■ #24 " 20 || fail "ctrl-o never returned to #24"
e2e_key C-o
e2e_wait "■ #9 " 20 || fail "ctrl-o never returned to #9"
e2e_key Escape '[105;5u'
e2e_wait "■ #24 " 20 || fail "ctrl-i never redid #24"
e2e_key Escape '[105;5u'
e2e_wait "■ #19 " 20 || fail "ctrl-i never redid #19"
e2e_shot history-redo
echo "PASS issue history"

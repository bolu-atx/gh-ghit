#!/bin/bash
# Fixture: default. Esc at the base opens the centered main menu. `s` opens
# Settings: the theme row applies live, the icons row flips, the backfill
# stepper steps. Esc closes each layer, and the menu lists the three entries.
source "$(dirname "$0")/lib.sh"
e2e_start
e2e_wait "move title" 60 || fail "pr data never landed"
e2e_key Escape
e2e_wait "Menu" 10 || fail "Esc at the base never opened the menu"
e2e_pane | grep -q "Settings" || fail "Settings entry missing"
e2e_pane | grep -q "Shortcuts" || fail "Shortcuts entry missing"
e2e_pane | grep -qE "Quit +q" || fail "Quit entry missing"
e2e_shot main-menu
e2e_key s
e2e_wait "backfill" 10 || fail "s never opened Settings"
e2e_pane | grep -q "all" || fail "backfill default not shown"
e2e_key j j h
e2e_wait "24 months" 10 || fail "backfill stepper did not step"
e2e_key k l
e2e_wait "plain" 10 || fail "icons row missing"
e2e_shot settings
e2e_key Escape
e2e_pane | grep -q "backfill" && fail "Esc left Settings open"
e2e_pane | grep -qE "Quit +q" && fail "menu stayed open"
e2e_key Escape
e2e_wait "Quit" 10 || fail "menu did not reopen"
e2e_key Escape
e2e_pane | grep -qE "Quit +q" && fail "Esc did not close the menu"
echo "PASS main menu"

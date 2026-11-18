#!/bin/bash
# Fixture: startup opens 9, 19, 24. Right-click on a sidebar row opens the
# actions menu; a click on "Pin" runs the pin and the row shows its marker.
# Mouse cells are 1-based like SGR; the pane scan is 0-based.
source "$(dirname "$0")/lib.sh"
e2e_start
e2e_bar 9
e2e_wait "/issues/9" 20 || fail "show #9"
e2e_wait "Issues 1" 20 || fail "sidebar never listed #9"
e2e_pane | grep -q "pinned (0)" || fail "row starts pinned"
# Locate the row by its number inside the 44-cell sidebar.
read -r row col < <(e2e_pane | python3 -c '
import sys
for i, line in enumerate(sys.stdin.read().splitlines()):
    at = line[:44].find("#9")
    if at >= 0:
        print(i + 1, at + 1)
        break
')
[ -n "${row:-}" ] || fail "sidebar row #9 not found"
e2e_key -l "$(printf '\033[<2;%s;%sM' "$col" "$row")"
e2e_key -l "$(printf '\033[<2;%s;%sm' "$col" "$row")"
e2e_wait "Evict row" 20 || fail "right-click never opened the menu"
e2e_shot menu-open
read -r row col < <(e2e_pane | python3 -c '
import re, sys
for i, line in enumerate(sys.stdin.read().splitlines()):
    m = re.search(r"Pin +p ", line)
    if m:
        print(i + 1, m.start() + 3)
        break
')
[ -n "${row:-}" ] || fail "Pin entry not found in the menu"
e2e_key -l "$(printf '\033[<0;%s;%sM' "$col" "$row")"
e2e_key -l "$(printf '\033[<3;%s;%sm' "$col" "$row")"
e2e_wait "pinned #9" 20 || fail "menu click never pinned"
e2e_wait "pinned \(1\)" 10 || fail "footer never counted the pin"
e2e_pane | grep -q "Evict row" && fail "menu stayed open after the click"
e2e_pane | grep -q $'\xef\x82\x8d' || fail "pin marker missing from the row"
e2e_shot menu-pinned
echo "PASS right-click menu pins a row"

#!/bin/bash
# Fixture: default listing.
# Opens the Format dialog and clicks the "author" option of the sort key
# row; the radio moves to it. The dialog fetches labels and milestones.
# fixtures: search
source "$(dirname "$0")/lib.sh"
e2e_start
e2e_wait "unifie" 60 || fail "pr rows never landed"
e2e_key f
e2e_wait "Format" 20 || fail "format dialog never opened"
e2e_wait "○ author" 10 || fail "sort options never rendered"
# Drive mouse cells are 1-based like SGR; the pane scan is 0-based.
read -r row col < <(e2e_pane | python3 -c '
import sys
for i, line in enumerate(sys.stdin.read().splitlines()):
    if "○ author" in line:
        print(i + 1, line.index("○ author") + 3)
        break
')
[ -n "${row:-}" ] || fail "author option not found"
e2e_key -l "$(printf '\033[<0;%s;%sM' "$col" "$row")"
e2e_key -l "$(printf '\033[<3;%s;%sm' "$col" "$row")"
e2e_wait "● author" 20 || fail "click never selected the author sort"
e2e_shot form-mouse
echo "PASS mouse picks a Format dialog option"

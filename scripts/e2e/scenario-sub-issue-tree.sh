#!/bin/bash
# Fixture: search hit #1001 is a parent with two REST sub-issues, one of
# them also a search hit and one outside the query. Tree mode and the
# all-time window come from prefs, so the mock search key has no dated
# token. Expanding the parent loads REST `issues/1001/sub_issues`.
# fixtures: tree
source "$(dirname "$0")/lib.sh"
mkdir -p "$E2E_XDG/ghit"
printf '{"sub_issue_tree": true, "merged_window_hours": 0}\n' > "$E2E_XDG/ghit/prefs.json"
e2e_start
e2e_wait "Precompiled" 60 || fail "startup listing never landed"
# g opens the Format dialog in query edit mode; Enter runs the search.
e2e_key g
e2e_type "label:bug"
e2e_key Enter
e2e_wait "search hit 1001" 30 || fail "search results never landed"
# The parent is expandable from its summary alone. Its children live
# only on GitHub, so it reads folded until the first click loads them.
read -r row col < <(e2e_pane | python3 -c '
import sys
for i, line in enumerate(sys.stdin.read().splitlines()):
    if "#1001" in line and "▸" in line:
        print(i, line.index("▸"))
        break
')
[ -n "${row:-}" ] || fail "#1001 folded glyph never rendered"
e2e_key -l "$(printf '\033[<0;%s;%sM' "$col" "$row")"
e2e_wait "outside query" 30 || fail "lazy sub-issues never landed"
e2e_wait "lazy child" 10 || fail "outside child row missing"
# The query hit nests under its parent once, at normal brightness, and
# the outside child follows it under the same connector column.
e2e_wait "├─  #1002" 10 || fail "query hit not nested under the parent"
e2e_wait "└─  #5001" 10 || fail "outside child not nested under the parent"
# A fold hides the loaded children; the session cache keeps them.
e2e_key -l "$(printf '\033[<0;%s;%sM' "$col" "$row")"
e2e_wait "▸ #1001" 10 || fail "glyph click never folded the loaded parent"
sleep 1
e2e_pane | grep -q "lazy child" && fail "fold kept the lazy child visible"
e2e_key -l "$(printf '\033[<0;%s;%sM' "$col" "$row")"
e2e_wait "lazy child" 10 || fail "cached children did not return on expand"
echo "PASS sub-issue tree"

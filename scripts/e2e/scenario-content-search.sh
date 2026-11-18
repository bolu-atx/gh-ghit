#!/bin/bash
# Fixture: issue 9's body is "Fixture body", so `body` matches twice in
# the Details text (the "Body" heading and the body line). With Main
# focused, `/` searches that content: the bar and then the toolbar show
# the counter, `n` steps it and Esc clears it. The sidebar keeps its row
# search.
source "$(dirname "$0")/lib.sh"
e2e_start
e2e_wait "move title" 60 || fail "startup listing never landed"
e2e_bar 9
e2e_wait "/issues/9" 20 || fail "show #9"
e2e_wait "Fixture body" 20 || fail "body never rendered"
# Main has no box: its tab rule turns double while no top border is double.
e2e_key Tab Tab
e2e_wait '═══' 10 || fail "Main never took focus"
e2e_pane | grep -q '╔' && fail "a side pane still has focus"

e2e_key /
e2e_type "body"
e2e_wait '/ body' 10 || fail "bar never showed the query"
e2e_wait '1/2' 10 || fail "bar never showed the 1/2 counter"
e2e_shot content-search-typing
e2e_key Enter
e2e_wait '‹ 1/2 ›' 10 || fail "toolbar never showed the counter"
e2e_wait 'n next match' 10 || fail "hints never showed n"
e2e_wait 'N previous match' 10 || fail "hints never showed N"
e2e_shot content-search-kept
e2e_key n
e2e_wait '‹ 2/2 ›' 10 || fail "n never moved to 2/2"
e2e_key n
e2e_wait '‹ 1/2 ›' 10 || fail "n never wrapped to 1/2"
e2e_key N
e2e_wait '‹ 2/2 ›' 10 || fail "N never wrapped back to 2/2"
e2e_shot content-search-next

# The counter is clickable: its right arrow steps to the next match.
click() { # $1 row, $2 col
  e2e_key -l "$(printf '\033[<0;%s;%sM' "$2" "$1")"
  e2e_key -l "$(printf '\033[<3;%s;%sm' "$2" "$1")"
}
read -r row col < <(e2e_pane | python3 -c '
import sys
for i, line in enumerate(sys.stdin.read().splitlines()):
    if "‹ 2/2 ›" in line:
        print(i + 1, line.index("‹ 2/2 ›") + 7)
        break
')
[ -n "${row:-}" ] || fail "counter not found on the tab line"
click "$row" "$col"
e2e_wait '‹ 1/2 ›' 10 || fail "clicking the counter arrow never stepped"

e2e_key Escape
for _ in {1..20}; do e2e_pane | grep -q '‹ 1/2 ›' || break; sleep 0.5; done
e2e_pane | grep -q '‹ 1/2 ›' && fail "Esc never cleared the search"
e2e_pane | grep -q 'n next match' && fail "Esc left the n hint"

# Esc in the bar cancels.
e2e_key /
e2e_type "body"
e2e_wait '1/2' 10 || fail "second search never counted"
e2e_key Escape
for _ in {1..20}; do e2e_pane | grep -q '/ body' || break; sleep 0.5; done
e2e_pane | grep -q '/ body' && fail "Esc in the bar never closed it"
e2e_pane | grep -q '‹' && fail "Esc in the bar kept a counter"

# Sidebar focus keeps the row search: the bar filters rows, no counter.
e2e_key BTab BTab
e2e_wait '╔[^╗]*Issues' 10 || fail "Issues never took focus"
e2e_key /
e2e_type "Precompiled"
e2e_wait 'instant|/ Precompiled' 10 || fail "row search bar never opened"
e2e_pane | grep -q '‹' && fail "row search showed a content counter"
e2e_key Escape
echo "PASS content search"

#!/bin/bash
# The dependency map, offline: the demo store (store/demo_tests.rs) is
# written through the real store code, every API call goes to a closed
# port, so this scenario needs no mock fixtures. Drag a node to pin it,
# drag empty space to pan, then unpin with x. Click a node for its card,
# click a link for the link card, click an end card, then Esc.
source "$(dirname "$0")/lib.sh"
export E2E_REPO=demo/graph E2E_NUMS=24 GHIT_API_BASE=http://127.0.0.1:9
host="$(printf '%s' "${GH_HOST:-github.com}" | tr '[:upper:]' '[:lower:]')"
GHIT_DEMO_STORE="$E2E_XDG/ghit/store/$host/demo/graph.sqlite" \
  cargo test --quiet --manifest-path "$E2E_ROOT/Cargo.toml" --bin ghit \
  store::demo_tests::write_graph_demo_store -- --ignored >/dev/null || fail "demo store not written"
e2e_start
e2e_wait "One local copy per repo" 60 || fail "stored issue never opened"
e2e_key 2
e2e_wait "items · linked to #24" 30 || fail "map never drew"
sleep 4
# Cell just inside the first `$1` right of the sidebar, 1-based "row col".
cell_of() {
  e2e_pane | python3 -c '
import sys
needle = sys.argv[1]
lines = sys.stdin.read().splitlines()
start = next(line.index("1 Details") for line in lines if "1 Details" in line) - 3
for i, line in enumerate(lines):
    if needle in line[start:]:
        print(i + 1, line.index(needle, start) + 2)
        break
' "$1"
}
mouse() { # $1 kind, $2 row, $3 col
  e2e_send "{\"type\":\"mouse\",\"kind\":\"$1\",\"column\":$3,\"row\":$2}"
  sleep 0.3
}
read -r row col < <(cell_of "#16")
[ -n "${row:-}" ] || fail "#16 not on the map"
mouse down-left "$row" "$col"
mouse drag-left "$((row + 4))" "$((col - 6))"
mouse drag-left "$((row + 8))" "$((col - 12))"
mouse up-left "$((row + 8))" "$((col - 12))"
e2e_wait "1 pinned" 10 || fail "drag never pinned the node"
e2e_wait "▌#16 Split the fetch queue" 10 || fail "the dragged node is not selected"
read -r row2 col2 < <(cell_of "#16 Split")
[ "$row2" = "$((row + 8))" ] || fail "node row $row2, want $((row + 8))"
# Pan: drag the blank end of the summary row; the pinned node follows.
read -r srow scol < <(cell_of "items · linked to #24")
mouse down-left "$srow" "$((scol + 50))"
mouse drag-left "$((srow - 3))" "$((scol + 44))"
mouse up-left "$((srow - 3))" "$((scol + 44))"
sleep 1
read -r row3 col3 < <(cell_of "#16 Split")
[ "$row3" = "$((row2 - 3))" ] && [ "$col3" = "$((col2 - 6))" ] || fail "pan moved #16 to $row3/$col3 from $row2/$col2"
e2e_key x
sleep 1
e2e_pane | grep -q "1 pinned" && fail "x left the node pinned"
# Card: a still click on a node shows its stored details at the right.
read -r row col < <(cell_of "#17 Fetch")
[ -n "${row:-}" ] || fail "#17 not on the map"
mouse down-left "$row" "$col"
mouse up-left "$row" "$col"
e2e_wait "┌ #17 " 10 || fail "no node card"
for text in "Fetch the current issue first" "issue · open · blocked" "milestone Fast startup" \
  "status" "labels    ● perf" "assignees @" "Demo item #17 for the dependency map" "[open ↵]"; do
  e2e_pane | grep -qF "$text" || fail "node card lacks: $text"
done
# 1-based "row col" of the middle of a run of three line cells left of
# the card, or "row col" of the first end card when $1 is "end".
card_spot() {
  e2e_pane | python3 -c '
import sys
lines = sys.stdin.read().splitlines()
start = next(line.index("1 Details") for line in lines if "1 Details" in line) - 3
top = next(i for i, line in enumerate(lines) if "┌ #" in line or "┌ link" in line)
card = lines[top].index("┌")
if sys.argv[1] == "end":
    print(top + 4, card + 4)
    sys.exit()
for i, line in enumerate(lines):
    for c in range(start + 1, min(card, len(line)) - 1):
        if line[c - 1:c + 2] == "───":
            print(i + 1, c + 1)
            sys.exit()
' "$1"
}
read -r erow ecol < <(card_spot edge)
[ -n "${erow:-}" ] || fail "no link cell on the map"
mouse down-left "$erow" "$ecol"
mouse up-left "$erow" "$ecol"
e2e_wait "┌ link " 10 || fail "no link card"
e2e_pane | grep -Eq "#[0-9]+ (blocks|closes|is the parent of) #[0-9]+" || fail "link card lacks the relation"
read -r prow pcol < <(card_spot end)
mouse down-left "$prow" "$pcol"
mouse up-left "$prow" "$pcol"
e2e_wait "[open ↵]" 10 || fail "an end card did not show its node card"
e2e_key Escape
sleep 1
e2e_pane | grep -qF "[next link e]" && fail "Esc left the card open"
# Physics dialog: P opens it, j x4 focuses growth direction, l turns it
# right -> down, r resets it, Esc closes it.
e2e_key P
e2e_wait "growth direction" 10 || fail "P did not open the physics dialog"
for _ in 1 2 3 4; do e2e_key j; done
e2e_key l
e2e_wait "−  down  +" 10 || fail "l did not turn the growth direction to down"
e2e_key r
e2e_wait "−  right  +" 10 || fail "r did not reset the growth direction"
e2e_key Escape
sleep 1
e2e_pane | grep -qF "growth direction" && fail "Esc left the physics dialog open"
e2e_send '{"type":"quit"}'
echo "PASS graph map"

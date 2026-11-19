#!/bin/bash
# The dependency map, offline: the demo store (store/demo_tests.rs) is
# written through the real store code, every API call goes to a closed
# port, so this scenario needs no mock fixtures. Drag a node to pin it,
# drag empty space to pan, then unpin with x.
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
e2e_send '{"type":"quit"}'
echo "PASS graph map"

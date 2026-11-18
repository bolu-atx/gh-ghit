#!/bin/bash
# Fixture: startup opens 9, 19, 24. `c` opens the comment editor in Write
# mode; typing fills it. Ctrl+R flips to Preview, where the markdown is
# rendered; a click on the Write word flips back. Ctrl+S sends one POST to
# the mock. Mouse cells are 1-based like SGR; the pane scan is 0-based.
source "$(dirname "$0")/lib.sh"
mock_log="$E2E_FRAMES/$E2E_SCENARIO.log"
e2e_start
e2e_bar 9
e2e_wait "/issues/9" 20 || fail "show #9"
e2e_wait "Issues 1" 20 || fail "sidebar never listed #9"
e2e_wait "comment" 20 || fail "toolbar missing comment"

click() { # $1 row, $2 col
  e2e_key -l "$(printf '\033[<0;%s;%sM' "$2" "$1")"
  e2e_key -l "$(printf '\033[<3;%s;%sm' "$2" "$1")"
}

e2e_key c
e2e_wait "Comment on #9" 20 || fail "c never opened the editor"
e2e_wait "Markdown works here" 5 || fail "placeholder missing"
e2e_type "# hello"
e2e_wait "7 chars · 1 line" 10 || fail "counter missing"
e2e_pane | grep -q "  1  # hello" || fail "draft not shown in Write mode"
e2e_shot compose-write

e2e_key C-r
e2e_wait "ctrl-r preview" 5 || fail "legend lost"
sleep 1
e2e_pane | grep -q "  1  # hello" && fail "Preview still shows the raw draft"
e2e_pane | grep -q "hello" || fail "Preview shows nothing"
e2e_shot compose-preview

# Click the idle Write word in the top border to flip back.
read -r row col < <(e2e_pane | python3 -c '
import sys
for i, line in enumerate(sys.stdin.read().splitlines()):
    if "Comment on #9" in line and "Write" in line:
        print(i + 1, line.index("Write") + 1)
        break
')
[ -n "${row:-}" ] || fail "switch not found"
click "$row" "$col"
e2e_wait "  1  # hello" 10 || fail "clicking Write never returned to the draft"
e2e_shot compose-back

grep -q "mock POST" "$mock_log" && fail "POST sent before ctrl-s"
e2e_key C-s
# The posted toast is replaced by the refetch toast, so wait on the POST.
for _ in $(seq 1 40); do grep -q "mock POST" "$mock_log" && break; sleep 0.5; done
grep -q 'mock POST /repos/bolu-atx/gh-ghit/issues/9/comments {"body": "# hello"}' "$mock_log" || fail "no comment POST recorded"
sleep 2
e2e_pane | grep -q "Comment on #9" && fail "editor still open after send"
echo "PASS comment editor flips modes and sends"

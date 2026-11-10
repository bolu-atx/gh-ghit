#!/bin/bash
# Unattended e2e for the pane-focus / yank / diff-line work.
# Live by design: fetches real data, so it needs network plus `gh` auth.
# Exit 0 = pass, 1 = fail. Never sends `o` (opens a browser).
set -u
S=ghit-e2e
REPO="${GHIT_E2E_REPO:-bolu-atx/gh-ghit}"
NUM="${GHIT_E2E_NUM:-19}"
cleanup() { tmux kill-session -t $S 2>/dev/null; }
trap cleanup EXIT
tmux kill-session -t $S 2>/dev/null
tmux new-session -d -s $S -x 200 -y 50 \
  "XDG_DATA_HOME=/tmp/ghit-e2e target/debug/ghit -R $REPO $NUM"
pane() { tmux capture-pane -t $S -p; }
wait_for() { # $1 pattern, $2 timeout in seconds
  for _ in $(seq 1 "$2"); do pane | grep -Eq "$1" && return 0; sleep 1; done
  return 1
}
markers() { pane | grep -o "█" | wc -l | tr -d ' '; }

wait_for "ghit" 60 || { echo "FAIL: app never rendered"; exit 1; }
# Focus starts on Issues; Tab moves it to PRs where #19 sits selected.
tmux send-keys -t $S Tab; sleep 1
[ "$(markers)" -eq 1 ] || { echo "FAIL: expected 1 focus marker on PRs, got $(markers)"; exit 1; }
# Tab on: at most one pane ever marks a row.
tmux send-keys -t $S Tab; sleep 1
[ "$(markers)" -le 1 ] || { echo "FAIL: marker leaked across panes"; exit 1; }
# Labeled dividers appear once recent rows land (requested + recent).
wait_for "─ (requested|recent) ─" 60 || { echo "FAIL: no labeled divider"; exit 1; }
# Diff tab: arrows walk lines, y toasts. The toast proves dispatch;
# a headless clipboard may answer "copy failed", which still passes.
tmux send-keys -t $S 3; sleep 1
wait_for "@@" 60 || { echo "FAIL: diff never loaded"; exit 1; }
tmux send-keys -t $S Down Down; sleep 1
tmux send-keys -t $S y; sleep 1
pane | grep -Eq "yanked|copy failed" || { echo "FAIL: no yank toast"; exit 1; }
tmux send-keys -t $S q; sleep 1
echo PASS

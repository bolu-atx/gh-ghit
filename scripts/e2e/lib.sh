#!/bin/bash
# e2e harness: tmux lifecycle, input, capture, waits, assertions.
# Scenarios source this file, then call e2e_start and drive. Never sends `o`.
set -u
E2E_REPO="${E2E_REPO:-bolu-atx/gh-ghit}"
E2E_NUMS="${E2E_NUMS:-9 19 24}"
E2E_XDG="${E2E_XDG:-/tmp/ghit-e2e-run}"
E2E_GEO="${E2E_GEO:-200x50}"
E2E_SESSION="${E2E_SESSION:-ghit-e2e}"
E2E_FRAMES="${E2E_FRAMES:-/tmp/ghit-e2e-frames}"
E2E_STEP="${E2E_STEP:-1}"
if [ -z "${E2E_ROOT:-}" ]; then
  E2E_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || true)"
  if [ -z "$E2E_ROOT" ]; then
    E2E_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
  fi
fi

# Boot a fresh detached session. Fails when the masthead never renders.
e2e_start() {
  mkdir -p "$E2E_FRAMES"
  trap 'tmux kill-session -t $E2E_SESSION 2>/dev/null' EXIT
  tmux kill-session -t $E2E_SESSION 2>/dev/null
  local geo_w="${E2E_GEO%x*}" geo_h="${E2E_GEO#*x}"
  tmux new-session -d -s $E2E_SESSION -x "$geo_w" -y "$geo_h" \
    "XDG_DATA_HOME=$E2E_XDG $E2E_ROOT/target/debug/ghit -R $E2E_REPO $E2E_NUMS"
  e2e_wait "ghit" 60 || fail "never rendered"
}

e2e_pane() { tmux capture-pane -t "$E2E_SESSION" -p; }
e2e_key() { tmux send-keys -t "$E2E_SESSION" "$@"; sleep "$E2E_STEP"; }
e2e_type() { tmux send-keys -t "$E2E_SESSION" -l "$1"; sleep "$E2E_STEP"; }
e2e_shot() { e2e_pane > "$E2E_FRAMES/$1.txt"; }

# Poll the visible pane until it matches (grep -E) or the timeout lapses.
e2e_wait() { # $1 pattern, $2 timeout seconds
  local end=$(( $(date +%s) + $2 ))
  while [ "$(date +%s)" -lt "$end" ]; do e2e_pane | grep -Eq "$1" && return 0; sleep 0.5; done
  return 1
}

e2e_markers() { e2e_pane | grep -o "█" | wc -l | tr -d ' '; }
e2e_toast() { e2e_pane | grep -o "yanked .\{0,78\}\|copy failed" | head -1; }

# Type text into the command bar and commit it.
e2e_bar() { # $1 text
  e2e_key :
  e2e_type "$1"
  e2e_key Enter
}

fail() { echo "FAIL [${E2E_SCENARIO:-?}] $1"; exit 1; }

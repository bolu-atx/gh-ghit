#!/bin/bash
# e2e harness: drive-mode lifecycle, input, capture, waits, assertions.
# Scenarios source this file, then call e2e_start and drive. Never sends `o`.
#
# Transport is the drive-mode Unix socket, not tmux: ghit boots headless
# with `--drive`, and drive.py bridges this shell to the session. The
# assertion surface is unchanged: e2e_pane prints the latest frame rows,
# e2e_wait greps them, markers and toasts read the same shapes.
set -u
E2E_REPO="${E2E_REPO:-bolu-atx/gh-ghit}"
E2E_NUMS="${E2E_NUMS:-9 19 24}"
E2E_XDG="${E2E_XDG:-/tmp/ghit-e2e-run}"
E2E_GEO="${E2E_GEO:-200x50}"
E2E_FRAMES="${E2E_FRAMES:-/tmp/ghit-e2e-frames}"
E2E_STEP="${E2E_STEP:-1}"
E2E_SOCK="${E2E_SOCK:-/tmp/ghit-e2e.sock}"
E2E_CMD="${E2E_CMD:-/tmp/ghit-e2e.cmd}"
E2E_PANE="${E2E_PANE:-$E2E_FRAMES/pane.txt}"
E2E_GHIT_PID=""
E2E_BRIDGE_PID=""
E2E_CMD_FD=""
# Stretch auto-refresh past any scenario lifetime: each scenario boots
# fresh and issues exactly one sections fetch instead of hammering the
# API every 5s (which also exhausts CI rate budget mid-run).
export GHIT_AUTO_REFRESH_SECS=3600
if [ -z "${E2E_ROOT:-}" ]; then
  E2E_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || true)"
  if [ -z "$E2E_ROOT" ]; then
    E2E_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
  fi
fi

e2e_cleanup() {
  [ -n "$E2E_BRIDGE_PID" ] && kill "$E2E_BRIDGE_PID" 2>/dev/null
  [ -n "$E2E_GHIT_PID" ] && kill "$E2E_GHIT_PID" 2>/dev/null
  [ -n "$E2E_CMD_FD" ] && eval "exec $E2E_CMD_FD>&-" 2>/dev/null
  rm -f "$E2E_SOCK" "$E2E_CMD"
}

# Boot headless on the socket and bridge into it. Fails when the
# masthead never renders.
e2e_start() {
  mkdir -p "$E2E_FRAMES"
  rm -f "$E2E_SOCK" "$E2E_CMD" "$E2E_PANE"
  mkfifo "$E2E_CMD"
  trap e2e_cleanup EXIT
  XDG_DATA_HOME="$E2E_XDG" "$E2E_ROOT/target/debug/ghit" \
    --drive "$E2E_SOCK" --size "$E2E_GEO" -R "$E2E_REPO" $E2E_NUMS &
  E2E_GHIT_PID=$!
  # Hold the write end open for the session so the bridge never sees EOF.
  # Fixed fd 9: automatic allocation (exec {var}<>) needs bash 4+, and
  # macOS ships bash 3.2, so the scenarios must run there too.
  exec 9<>"$E2E_CMD"
  E2E_CMD_FD=9
  python3 "$E2E_ROOT/scripts/e2e/drive.py" --sock "$E2E_SOCK" \
    --cmd "$E2E_CMD" --pane "$E2E_PANE" --log "$E2E_FRAMES/frames.jsonl" &
  E2E_BRIDGE_PID=$!
  e2e_wait "ghit" 60 || fail "never rendered"
}

e2e_pane() { cat "$E2E_PANE" 2>/dev/null; }

# One protocol line into the session.
e2e_send() { printf '%s\n' "$1" >&$E2E_CMD_FD; }

# tmux-style key name to one drive-protocol line.
e2e_key_line() { # $1 key name
  case "$1" in
    Enter) printf '{"type":"key","key":"Enter"}\n' ;;
    Escape) printf '{"type":"key","key":"Esc"}\n' ;;
    Tab) printf '{"type":"key","key":"Tab"}\n' ;;
    Space) printf '{"type":"key","key":" "}\n' ;;
    BSpace|Backspace) printf '{"type":"key","key":"Backspace"}\n' ;;
    Up|Down|Left|Right|PageDown|PageUp|Home|End|Delete) printf '{"type":"key","key":"%s"}\n' "$1" ;;
    C-o) printf '{"type":"key","key":"o","ctrl":true}\n' ;;
    C-c) printf '{"type":"key","key":"c","ctrl":true}\n' ;;
    C-d) printf '{"type":"key","key":"d","ctrl":true}\n' ;;
    \[*\;5u) # kitty CSI-u release, e.g. [105;5u for Ctrl+I redo.
      local code="${1#\[}"; code="${code%%;*}"
      # shellcheck disable=SC2059
      local ch; printf -v ch "\\$(printf '%03o' "$code")"
      printf '{"type":"key","key":"%s","ctrl":true}\n' "$ch" ;;
    ?) # single printable char, JSON-escaped.
      local ch="$1"
      ch="${ch//\\/\\\\}"; ch="${ch//\"/\\\"}"
      printf '{"type":"key","key":"%s"}\n' "$ch" ;;
    *) echo "unknown e2e key: $1" >&2; return 1 ;;
  esac
}

# SGR mouse report (1-based cells) to one drive-protocol line.
e2e_mouse_line() { # $1 raw sequence like $'\x1b[<0;4;25M'
  local seq="$1" kind column row btn rest
  seq="${seq#*$'\x1b[<'}"
  btn="${seq%%;*}"; rest="${seq#*;}"
  column="${rest%%;*}"; rest="${rest#*;}"
  row="${rest%[Mm]}"
  local press="${rest: -1}"
  case "$btn" in
    64) kind="scroll-up" ;;
    65) kind="scroll-down" ;;
    3|*m) kind="up-left" ;;
    *) if [ "$press" = "m" ]; then kind="up-left"; else kind="down-left"; fi ;;
  esac
  # SGR release uses lowercase m regardless of button.
  [ "$press" = "m" ] && kind="up-left"
  printf '{"type":"mouse","kind":"%s","column":%s,"row":%s}\n' "$kind" "$column" "$row"
}

e2e_key() {
  while [ $# -gt 0 ]; do
    case "$1" in
      -l) e2e_send "$(e2e_mouse_line "$2")"; shift 2 ;;
      *) e2e_send "$(e2e_key_line "$1")"; shift ;;
    esac
    sleep "$E2E_STEP"
  done
}
e2e_type() {
  local text="$1" i ch
  for (( i=0; i<${#text}; i++ )); do
    ch="${text:$i:1}"
    ch="${ch//\\/\\\\}"; ch="${ch//\"/\\\"}"
    e2e_send "{\"type\":\"key\",\"key\":\"$ch\"}"
  done
  sleep "$E2E_STEP"
}
e2e_shot() { e2e_pane > "$E2E_FRAMES/$1.txt"; }

# Poll the latest frame until it matches (grep -E) or the timeout lapses.
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

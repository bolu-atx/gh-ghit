#!/bin/bash
# Run every e2e scenario in its own drive-mode session. Exit 0 only when all pass.
#
# Headless CI has no clipboard tool, so yank scenarios would only ever see
# "copy failed". With E2E_FAKE_CLIPBOARD=1 (set by e2e.yml) PATH gets a
# shim that accepts the copy, and the yank toasts assert on content.
set -u
cd "$(dirname "$0")"
export E2E_FRAMES="${E2E_FRAMES:-/tmp/ghit-e2e-frames}"
export E2E_TIMEOUT="${E2E_TIMEOUT:-180}"
if [ "${E2E_FAKE_CLIPBOARD:-0}" = "1" ]; then
  E2E_SHIM="${E2E_SHIM:-/tmp/ghit-e2e-bin}"
  mkdir -p "$E2E_SHIM"
  for tool in pbcopy wl-copy xclip xsel; do
    printf '#!/bin/sh\ncat >/dev/null\nexit 0\n' > "$E2E_SHIM/$tool"
    chmod +x "$E2E_SHIM/$tool"
  done
  export PATH="$E2E_SHIM:$PATH"
fi
rm -rf "$E2E_FRAMES"; mkdir -p "$E2E_FRAMES"
export GH_TOKEN=e2e-fake GH_HOST=invalid.example
mock_pid=""
cleanup() { [ -z "$mock_pid" ] || kill "$mock_pid" 2>/dev/null || true; }
trap cleanup EXIT
pass=0; failed=0; failed_names=""
for s in scenario-*.sh; do
  export E2E_SCENARIO="$s" E2E_XDG="$E2E_FRAMES/xdg-$s"
  echo "=== $s"
  fixture_set="$(sed -n 's/^# fixtures: //p' "$s" | head -1)"
  fixture_set="${fixture_set:-${E2E_FIXTURES:-default}}"
  if [ "${E2E_RECORD:-0}" = 1 ] && [ "$fixture_set" != default ]; then
    echo "--- SKIP $s (injected fixture: $fixture_set)"
    continue
  fi
  port_file="$E2E_FRAMES/mock-port"
  rm -f "$port_file"
  if [ "${E2E_RECORD:-0}" = 1 ]; then
    python3 mock_github.py --port-file "$port_file" --fixtures "$fixture_set" \
      --record >> "$E2E_FRAMES/$s.log" 2>&1 &
  else
    python3 mock_github.py --port-file "$port_file" --fixtures "$fixture_set" \
      >> "$E2E_FRAMES/$s.log" 2>&1 &
  fi
  mock_pid=$!
  deadline=$(( $(date +%s) + 10 ))
  while [ ! -s "$port_file" ]; do
    if ! kill -0 "$mock_pid" 2>/dev/null || [ "$(date +%s)" -ge "$deadline" ]; then
      echo "mock server failed or timed out (see $E2E_FRAMES/$s.log)" >&2
      head -1 "$E2E_FRAMES/$s.log" >&2
      exit 1
    fi
    sleep 0.1
  done
  export GHIT_API_BASE="http://127.0.0.1:$(cat "$port_file")"
  if timeout "$E2E_TIMEOUT" bash "$s" >> "$E2E_FRAMES/$s.log" 2>&1 &&
     ! grep -q "unmatched mock request:" "$E2E_FRAMES/$s.log"; then
    echo "--- PASS $s"; pass=$((pass + 1))
  else
    echo "--- FAIL $s (see $E2E_FRAMES/$s.log)"; tail -4 "$E2E_FRAMES/$s.log"
    failed=$((failed + 1)); failed_names="$failed_names $s"
  fi
  cleanup
  wait "$mock_pid" 2>/dev/null || true
  mock_pid=""
done
echo "scenarios: $pass passed, $failed failed$failed_names"
[ "$failed" -eq 0 ]

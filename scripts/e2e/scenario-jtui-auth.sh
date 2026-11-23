#!/bin/bash
# jtui: auth status reports env and ok; a rejected token exits with the 401 message
# and never opens a drive-mode socket.
source "$(dirname "$0")/lib.sh"
export E2E_SOCK=/tmp/jtui-auth.sock E2E_PANE="$E2E_FRAMES/jtui-auth-pane.txt"
export XDG_CONFIG_HOME="$E2E_FRAMES/jtui-auth-config"
e2e_jira_start
mkdir -p "$XDG_CONFIG_HOME/jtui"
cat > "$XDG_CONFIG_HOME/jtui/config.toml" <<EOF
[repos."$E2E_REPO"]
site = "fixture.atlassian.net"
project = "PROJ"
email = "$JIRA_E2E_EMAIL"
EOF
out="$E2E_FRAMES/jtui-status.out"
XDG_DATA_HOME="$E2E_XDG" timeout 30 "$E2E_ROOT/target/debug/jtui" \
  auth status -R "$E2E_REPO" >"$out" 2>&1 || fail "auth status: $(cat "$out")"
grep -q '^token source: env$' "$out" || fail "no env source: $(cat "$out")"
grep -q '^ok$' "$out" || fail "no ok: $(cat "$out")"
! grep -q "$JIRA_API_TOKEN" "$out" || fail "auth status printed the token"
export JIRA_API_TOKEN=e2e-wrong-token
rm -f "$E2E_SOCK"
out="$E2E_FRAMES/jtui-auth.out"
XDG_DATA_HOME="$E2E_XDG" timeout 30 "$E2E_ROOT/target/debug/jtui" \
  --drive "$E2E_SOCK" -R "$E2E_REPO" >"$out" 2>&1
status=$?
[ "$status" -eq 1 ] || fail "wrong token exited $status, want 1: $(cat "$out")"
grep -q "Jira rejected the email or JIRA_API_TOKEN (401/403)" "$out" ||
  fail "no 401 message: $(cat "$out")"
[ ! -e "$E2E_SOCK" ] || fail "a TUI started on the socket"
echo "PASS jtui auth"

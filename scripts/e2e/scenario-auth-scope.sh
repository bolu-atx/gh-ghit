#!/bin/bash
# Fixture: none — auth fails before any fetch matters. A dead token must
# raise the re-auth modal naming scopes and the refresh command, and `n`
# must dismiss to a status-line hint. Never sends `o`.
source "$(dirname "$0")/lib.sh"
export GH_TOKEN="ghit-e2e-bad-token"
export E2E_XDG=/tmp/ghit-e2e-auth E2E_SOCK=/tmp/ghit-auth.sock
export E2E_CMD=/tmp/ghit-auth.cmd E2E_PANE="$E2E_FRAMES/auth-pane.txt"
e2e_start
e2e_wait "GitHub auth" 60 || fail "auth modal never rose"
e2e_wait "gh auth refresh -s repo -s read:org -s project" 10 || fail "no re-auth command"
e2e_shot auth-modal
e2e_key n
# The dismiss hint yields to the landing fetch error, which must carry
# the same command: nothing about auth ever fails silently.
e2e_wait "re-auth with more permissions" 20 || fail "fetch error names no fix"
echo "hint: $(e2e_pane | grep -o 're-auth with more permissions.*' | head -1)"
echo "PASS auth scope"

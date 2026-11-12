#!/bin/bash
# fixtures: scope-gap
# A token missing required scopes must show the exact gap.
source "$(dirname "$0")/lib.sh"
export E2E_SOCK=/tmp/ghit-scope-gap.sock
export E2E_CMD=/tmp/ghit-scope-gap.cmd E2E_PANE="$E2E_FRAMES/scope-gap-pane.txt"
e2e_start
e2e_wait "missing scopes: read:org, project" 60 || fail "missing scope gap"
e2e_wait "gh auth refresh -s repo -s read:org -s project" 10 || fail "no re-auth command"
e2e_shot scope-gap-modal
echo "PASS scope gap"

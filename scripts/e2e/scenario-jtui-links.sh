#!/bin/bash
# jtui: PROJ-1 carries Blocks, Relates and out-of-project links in the
# fixture. Opening it shows the Links block grouped by link name.
source "$(dirname "$0")/lib.sh"
export E2E_BIN=jtui E2E_NUMS=""
export E2E_SOCK=/tmp/jtui-links.sock E2E_CMD=/tmp/jtui-links.cmd
export E2E_PANE="$E2E_FRAMES/jtui-links-pane.txt"
export XDG_CONFIG_HOME="$E2E_FRAMES/jtui-links-config"
e2e_jira_start
mkdir -p "$XDG_CONFIG_HOME/jtui"
cat > "$XDG_CONFIG_HOME/jtui/config.toml" <<EOF
[repos."$E2E_REPO"]
site = "fixture.atlassian.net"
project = "PROJ"
email = "$JIRA_E2E_EMAIL"
EOF
e2e_start
e2e_wait "PROJ-1" 60 || fail "the Jira rows never landed"
read -r row col < <(e2e_pane | python3 -c '
import sys
for i, line in enumerate(sys.stdin.read().splitlines()):
    if "PROJ-1" in line:
        print(i + 1, line.index("PROJ-1") + 1)
        break
')
[ -n "${row:-}" ] || fail "PROJ-1 row never rendered"
e2e_key -l "$(printf '\033[<0;%s;%sM' "$col" "$row")"
e2e_wait "Links" 30 || fail "the Links block never showed"
e2e_wait "is blocked by" 10 || fail "the blocked-by group is missing"
e2e_wait "PROJ-2" 10 || fail "the blocks target is missing"
e2e_wait "PROJ-4" 10 || fail "the relates target is missing"
e2e_wait "OTHER-12" 10 || fail "the out-of-project target is missing"
echo "PASS jtui links"

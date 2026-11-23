#!/bin/bash
# jtui: Jira issues from both search pages of the Jira fixture, GitHub
# PRs from the default GitHub fixture. GitHub issue rows must stay out.
source "$(dirname "$0")/lib.sh"
export E2E_BIN=jtui E2E_NUMS=""
export E2E_SOCK=/tmp/jtui-basic.sock E2E_CMD=/tmp/jtui-basic.cmd
export E2E_PANE="$E2E_FRAMES/jtui-basic-pane.txt"
export XDG_CONFIG_HOME="$E2E_FRAMES/jtui-basic-config"
# The mock exports JIRA_API_TOKEN and JIRA_E2E_EMAIL from its auth block.
e2e_jira_start
mkdir -p "$XDG_CONFIG_HOME/jtui"
cat > "$XDG_CONFIG_HOME/jtui/config.toml" <<EOF
[repos."$E2E_REPO"]
site = "fixture.atlassian.net"
project = "PROJ"
email = "$JIRA_E2E_EMAIL"
EOF
e2e_start
e2e_wait "Issues 4" 60 || fail "the issue sections never took the Jira rows"
e2e_wait "PROJ-1" 10 || fail "first search page is incomplete"
e2e_wait "PROJ-2" 10 || fail "first search page is incomplete"
# PROJ-3 and PROJ-4 ride page 2: their rows prove nextPageToken paging.
e2e_wait "PROJ-3" 10 || fail "second search page never landed"
e2e_wait "PROJ-4" 10 || fail "second search page is incomplete"
e2e_wait "Pull requests 3" 30 || fail "the PR section never took the GitHub PRs"
e2e_wait "unified interface" 10 || fail "GitHub PR rows never landed"
e2e_wait "#19" 10 || fail "the PR numbers are not GitHub numbers"
e2e_pane | grep -q "Precompiled binaries" && fail "a GitHub issue row showed"
e2e_pane | grep -q "#9 " && fail "the GitHub issue number showed"
echo "PASS jtui basic"

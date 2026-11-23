#!/bin/bash
# jtui: an uncovered `/` query runs both search arms. The Jira arm sends
# the label term as JQL, the GitHub arm searches PRs, and the two result
# sets list together. A GitHub issue hit stays out of the Jira issues
# pane.
# fixtures: jtui-filters
source "$(dirname "$0")/lib.sh"
export E2E_BIN=jtui E2E_NUMS=""
export E2E_SOCK=/tmp/jtui-search.sock E2E_CMD=/tmp/jtui-search.cmd
export E2E_PANE="$E2E_FRAMES/jtui-search-pane.txt"
export XDG_CONFIG_HOME="$E2E_FRAMES/jtui-search-config"
e2e_jira_start
mkdir -p "$XDG_CONFIG_HOME/jtui"
cat > "$XDG_CONFIG_HOME/jtui/config.toml" <<EOF
[repos."$E2E_REPO"]
site = "fixture.atlassian.net"
project = "PROJ"
email = "$JIRA_E2E_EMAIL"
EOF
e2e_start
e2e_wait "Issues 4" 60 || fail "the Jira issues never landed"
e2e_wait "Pull requests 3" 30 || fail "the GitHub PRs never landed"
e2e_key f
e2e_wait "Format" 10 || fail "format dialog never opened"
# All time: no `updated:` token, so the fixture keys match exactly.
e2e_key 4
e2e_key e
e2e_type "label:bug"
e2e_key Enter
e2e_wait "searches GitHub" 10 || fail "the unanswerable query did not name the action"
e2e_key H
e2e_wait "PROJ-1" 30 || fail "the Jira arm returned no issue for label:bug"
e2e_wait "unified interface" 10 || fail "the GitHub arm returned no PR"
e2e_wait "GitHub \+ Jira" 10 || fail "the bar does not name both sources"
e2e_wait "Issues \[query\] 1" 10 || fail "the Jira search result count is wrong"
e2e_wait "Pull requests \[query\] 1" 10 || fail "the GitHub search result count is wrong"
e2e_pane | grep -q "PROJ-2" && fail "an unmatched Jira issue was listed"
e2e_pane | grep -q "search hit 1001" && fail "a GitHub issue hit reached the issues pane"
echo "PASS jtui search"

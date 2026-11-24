#!/bin/bash
# jtui: the Labels pane lists Jira labels and fix versions beside the
# GitHub labels, one list deduped by name, with a marker on a name only
# one source lists. Toggling `bug` narrows the Jira issues and the GitHub
# PRs; a fix-version pill narrows the Jira issues.
# fixtures: jtui-filters
source "$(dirname "$0")/lib.sh"
export E2E_BIN=jtui E2E_NUMS=""
export E2E_SOCK=/tmp/jtui-filters.sock E2E_CMD=/tmp/jtui-filters.cmd
export E2E_PANE="$E2E_FRAMES/jtui-filters-pane.txt"
export XDG_CONFIG_HOME="$E2E_FRAMES/jtui-filters-config"
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
e2e_key Tab Tab Tab
e2e_wait "Filters" 10 || fail "labels pane missing"
e2e_wait "checkout jira" 20 || fail "a Jira-only label never reached the pane"
e2e_pane | grep -q "docs github" || fail "a GitHub-only label carries no marker"
e2e_pane | grep -q "bug jira\|bug github" && fail "a label both sources list carries a marker"
e2e_wait "1.0 jira" 10 || fail "the fix-version pill is missing"
e2e_shot jtui-filters-labels
# Rows: 1.0, 2.0, (no milestone), bug, checkout, chore, feature, ui, docs.
e2e_key j j j
e2e_key Space
e2e_wait "1 on" 10 || fail "the bug label never checked"
e2e_wait "Issues 1" 10 || fail "the label did not narrow the Jira issues"
e2e_wait "Pull requests 1" 10 || fail "the labelled GitHub PR vanished"
e2e_pane | grep -q "PROJ-1" || fail "the labelled Jira issue vanished"
e2e_pane | grep -q "PROJ-2" && fail "an unlabelled Jira issue survived the label"
e2e_key X
e2e_wait "Issues 4" 10 || fail "clear all did not restore the Jira issues"
# Back up to the 1.0 pill: only the Jira issues of that fix version stay.
e2e_key k k k
e2e_key Space
e2e_wait "Issues 2" 10 || fail "the fix-version pill did not narrow the Jira issues"
e2e_pane | grep -q "PROJ-1" || fail "PROJ-1 left its fix version"
e2e_pane | grep -q "PROJ-4" && fail "a Jira issue of another fix version survived"
echo "PASS jtui filters"

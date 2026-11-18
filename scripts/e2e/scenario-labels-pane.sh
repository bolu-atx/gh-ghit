#!/bin/bash
# Fixture: only issue #9 carries the `bug` label, and PR #24 closes #9, so
# the right column shows Refs plus the Labels pane. Tab x3 reaches Labels,
# space on `bug` must drop the unlabelled recent PR #25 from the sidebar
# (requested #19 and #24 stay), X must restore it.
# fixtures: labels
source "$(dirname "$0")/lib.sh"
e2e_start
e2e_wait "move title" 60 || fail "pr data never landed"
e2e_bar 24
e2e_wait "/pull/24" 20 || fail "show #24"
e2e_key Tab Tab Tab
e2e_wait "Filters" 10 || fail "labels pane missing"
e2e_wait "█ +.? \(no milestone\)" 20 || fail "meta never loaded"
e2e_shot labels-focus
e2e_pane | grep -q "bug" || fail "bug label row missing"
e2e_pane | grep -q "Pull requests 3" || fail "expected 3 PRs before filter"
# Rows: (no milestone), bug, docs. Move onto bug and toggle it.
e2e_key j Space
e2e_wait "labels \(1/2\)" 10 || fail "bug never checked"
e2e_wait "1 on" 10 || fail "active count missing"
e2e_shot labels-bug
e2e_pane | grep -q "Precompiled" || fail "labelled issue #9 vanished"
e2e_pane | grep -q "Pull requests 2" || fail "unlabelled PR #25 survived the filter"
e2e_key X
e2e_wait "Pull requests 3" 10 || fail "clear did not restore the sidebar"
e2e_pane | grep -qF "milestones (0/1) · labels (0/2)" || fail "counts not cleared"
echo "PASS labels pane"

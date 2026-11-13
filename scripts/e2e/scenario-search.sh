#!/bin/bash
# Fixture: 102 paginated search hits (two pages) plus the labels and
# milestones pickers. All-time keeps the search query free of a dated
# `updated:` token, so the mock keys stay deterministic.
# fixtures: search
source "$(dirname "$0")/lib.sh"
e2e_start
e2e_wait "Precompiled" 60 || fail "startup listing never landed"
# The sidebar Format dialog loads the picker lists lazily.
e2e_key f
e2e_wait "Format" 10 || fail "format dialog never opened"
e2e_wait "bug" 20 || fail "labels never loaded"
e2e_wait "v0.6" 20 || fail "milestones never loaded"
# All time: no `updated:` token, so the fixture keys match exactly.
e2e_key 4
e2e_key e
e2e_type "label:bug"
e2e_key Enter
# 3 requested rows (9 is an issue) plus all 102 issue hits prove page 2
# landed: the Issues pane counts both pages.
e2e_wait "103 ·" 30 || fail "paged search never landed"
# Label pills narrow loaded rows without a refetch or a query edit.
e2e_key f
for _ in {1..7}; do e2e_key j; done
e2e_key Space
e2e_key Escape
e2e_wait '\[query\] 1 · filtered' 10 || fail "label pill did not narrow"
e2e_key f
e2e_key Space
e2e_key Escape
e2e_wait "103 ·" 10 || fail "label pill did not restore"
echo "PASS search"

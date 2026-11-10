#!/bin/bash
# Fixture: 24 is a PR; its title renders in the masthead once fetched.
source "$(dirname "$0")/lib.sh"
e2e_start
e2e_wait "move title" 60 || fail "pr data never landed"
e2e_bar 24
e2e_wait "■ #24 " 20 || fail "show #24"
e2e_key y
e2e_shot pr-yank
e2e_toast | grep -q "yanked #24" || fail "pr yank toast: $(e2e_toast)"
echo "toast: $(e2e_toast)"
echo "PASS pr yank"

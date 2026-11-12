#!/bin/bash
# Record the requests made by the scenarios with a developer's GitHub token.
set -eu
cd "$(dirname "$0")"
token="${GH_TOKEN:-${GITHUB_TOKEN:-}}"
if [ -z "$token" ]; then token="$(gh auth token)"; fi
[ -n "$token" ] || { echo "GitHub token required" >&2; exit 1; }
# The auth scenario must retain its injected failure, so record only normal scenarios.
export E2E_RECORD_TOKEN="$token"
export E2E_RECORD=1 E2E_FAKE_CLIPBOARD="${E2E_FAKE_CLIPBOARD:-1}"
exec bash run.sh

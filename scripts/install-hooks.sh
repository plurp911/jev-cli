#!/bin/sh
# Install this repository's tracked hooks in the current clone only.
set -eu

repo_root=$(git rev-parse --show-toplevel)
cd "$repo_root"

current=$(git config --local --get core.hooksPath || true)
if [ -n "$current" ] && [ "$current" != .githooks ]; then
  printf 'core.hooksPath is already %s; preserve that hook setup and install this hook manually.\n' "$current" >&2
  exit 1
fi

git config --local core.hooksPath .githooks
printf 'Installed local pre-push hook: scripts/verify.sh --push\n'

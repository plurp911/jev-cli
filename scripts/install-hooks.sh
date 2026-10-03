#!/bin/sh
# Install this repository's tracked hooks in the current clone only.
set -eu

repo_root=$(git rev-parse --show-toplevel)
cd "$repo_root"

current=$(git config --local --get core.hooksPath || true)
if [ -n "$current" ]; then
  # Resolve existing directories, including relative/trailing-slash and macOS
  # symlink spellings. Git Bash also accepts Windows paths after MSYS conversion.
  candidate=$current
  if command -v cygpath >/dev/null 2>&1; then
    candidate=$(cygpath -u "$candidate")
  fi
  case "$candidate" in
    /*) ;;
    *) candidate=$repo_root/$candidate ;;
  esac
  actual=$(CDPATH='' cd "$candidate" 2>/dev/null && pwd -P) || actual=''
  expected=$(CDPATH='' cd "$repo_root/.githooks" 2>/dev/null && pwd -P) || expected=''
  if [ -z "$actual" ] || [ "$actual" != "$expected" ]; then
    printf 'core.hooksPath is already %s; preserve that hook setup and install this hook manually.\n' "$current" >&2
    exit 1
  fi
fi

git config --local core.hooksPath .githooks
printf 'Installed local pre-push hook: scripts/verify.sh --push\n'

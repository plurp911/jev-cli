#!/usr/bin/env bash
# Provision the skill-authoring development toolchain.
#
# This installs the *development* agent skills used to author and evaluate the skills
# this repository ships. It is not part of building, testing, or running `jev`, and the
# `jev` binary gains no dependency from it.
#
# What it does, and nothing else:
#
#   1. Clones each upstream repository named in `skill-authoring-lock.json` at its
#      pinned commit into `references/07-skill-authoring/`, which is gitignored.
#   2. Verifies each clone is at exactly that commit.
#   3. Copies the listed skill directories into `.claude/skills/`, verbatim, each with
#      the upstream licence file and a generated `PROVENANCE.md`.
#
# What it deliberately does not do:
#
#   * It runs no code from the clones. Not an installer, not a test suite, not a build.
#     `git clone` does not execute remote hooks, and nothing here invokes anything from
#     a cloned tree.
#   * It touches no global configuration. Nothing is written outside this repository.
#   * It downloads no tarball, pipes nothing into a shell, and installs no package.
#
# Usage:
#   scripts/skill-authoring-setup.sh           # install or update to the pinned state
#   scripts/skill-authoring-setup.sh --check   # verify only, no network, no writes
#   scripts/skill-authoring-setup.sh --list    # print what the lockfile pins
set -Eeuo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

LOCK="skill-authoring-lock.json"
MODE="install"
case "${1:-}" in
  --check) MODE="check" ;;
  --list)  MODE="list" ;;
  --help|-h) sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
  "") ;;
  *) printf 'unknown argument: %s\n' "$1" >&2; exit 2 ;;
esac

if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
  BOLD=$'\033[1m'; RED=$'\033[31m'; GREEN=$'\033[32m'; YELLOW=$'\033[33m'; OFF=$'\033[0m'
else
  BOLD=""; RED=""; GREEN=""; YELLOW=""; OFF=""
fi

fail() { printf '%s!! %s%s\n' "$RED" "$*" "$OFF" >&2; exit 1; }

command -v git >/dev/null 2>&1 || fail "git is required"
command -v python3 >/dev/null 2>&1 || fail "python3 is required"
[ -f "$LOCK" ] || fail "$LOCK not found; run this from the repository"

# The lockfile is the single declaration of what is pinned. Reading it with python3
# rather than hand-rolling JSON parsing in shell keeps the two in step: a malformed
# lockfile fails here instead of silently installing something else.
#
# `while ... done < <(read_lock ...)` would NOT keep that promise: `set -e` does not see
# a process substitution fail, so a lockfile entry missing a key would print a traceback,
# feed the loop fewer rows, and let the script report success over a partial install.
# `read_lock_or_fail` captures the output and the status, so a bad lockfile stops here.
read_lock() { python3 - "$LOCK" "$1" <<'PY'
import json, sys
lock = json.load(open(sys.argv[1], encoding="utf-8"))
what = sys.argv[2]
sources = lock["sources"]
if what == "sources":
    for slug, s in sources.items():
        print("\t".join([slug, s["url"], s["commit"], s["clone"], s["license"]]))
elif what == "installs":
    for slug, s in sources.items():
        for item in s.get("install", []):
            print("\t".join([slug, s["clone"], item["from"], item["to"],
                             item["license"], s["license"], s["commit"], s["url"]]))
PY
}

read_lock_or_fail() {
  local out
  # No `|| true` and no process substitution at the call sites: this runs in a command
  # substitution, so a non-zero status here is a non-zero status for the assignment, and
  # `set -e` stops the script. Reading through `< <(...)` instead would swallow it --
  # `exit` inside a process substitution leaves only that subshell, and the loop would
  # simply see fewer rows and the script would report success over a partial install.
  out="$(read_lock "$1")" || fail "$LOCK could not be read (see the error above)"
  [ -n "$out" ] || fail "$LOCK produced no $1"
  # Tab and newline are the field and record separators below, so an empty or
  # separator-bearing value would shift every field after it. Nothing legitimate has one.
  case "$out" in
    *"$(printf '\t\t')"*) fail "$LOCK has an empty field in its $1" ;;
  esac
  printf '%s\n' "$out"
}

# Read both tables up front, in the current shell, so a malformed lockfile stops the
# script before anything is cloned or deleted.
LOCK_SOURCES="$(read_lock_or_fail sources)"
LOCK_INSTALLS="$(read_lock_or_fail installs)"

if [ "$MODE" = "list" ]; then
  printf '%sPinned upstream sources%s\n' "$BOLD" "$OFF"
  while IFS=$'\t' read -r slug url commit clone license; do
    printf '  %-36s %s  %s\n' "$slug" "${commit:0:12}" "$license"
    printf '    %s -> %s\n' "$url" "$clone"
  done <<< "$LOCK_SOURCES"
  printf '\n%sInstalled development skills%s\n' "$BOLD" "$OFF"
  while IFS=$'\t' read -r slug clone from to _lic _slic _commit _url; do
    printf '  %-40s from %s/%s\n' "$to" "$slug" "$from"
  done <<< "$LOCK_INSTALLS"
  exit 0
fi

# --- 1. Clones, at exactly the pinned commit. -------------------------------------

while IFS=$'\t' read -r slug url commit clone _license; do
  if [ ! -d "$clone/.git" ]; then
    if [ "$MODE" = "check" ]; then
      fail "$clone is missing; run scripts/skill-authoring-setup.sh"
    fi
    printf '%s==> cloning %s%s\n' "$BOLD" "$slug" "$OFF"
    mkdir -p "$(dirname "$clone")"
    git clone --no-checkout --no-recurse-submodules --quiet "$url" "$clone"
  fi

  have="$(git -C "$clone" rev-parse HEAD 2>/dev/null || echo none)"
  if [ "$have" != "$commit" ]; then
    if [ "$MODE" = "check" ]; then
      fail "$clone is at ${have:0:12}, lockfile pins ${commit:0:12}"
    fi
    printf '%s==> fetching %s at %s%s\n' "$BOLD" "$slug" "${commit:0:12}" "$OFF"
    git -C "$clone" fetch --quiet --no-recurse-submodules origin "$commit" \
      || git -C "$clone" fetch --quiet --no-recurse-submodules origin
    git -C "$clone" checkout --quiet --detach "$commit"
  fi

  have="$(git -C "$clone" rev-parse HEAD)"
  [ "$have" = "$commit" ] || fail "$clone did not reach $commit (at $have)"
  printf '%s  ok  %s @ %s%s\n' "$GREEN" "$slug" "${commit:0:12}" "$OFF"
done <<< "$LOCK_SOURCES"

# --- 2. Development skills, copied verbatim with their licence and provenance. -----
#
# Verbatim matters twice over. Under Apache-2.0 §4 and the MIT notice clause an
# unmodified copy needs no "modified this file" notice, and a skill we edited would no
# longer be the upstream methodology we pinned -- it would be our own fork wearing the
# upstream's name, and the next update would silently discard the edit.

while IFS=$'\t' read -r slug clone from to lic slic commit url; do
  src="$clone/$from"
  [ -d "$src" ] || fail "$src not found in the pinned checkout of $slug"

  # The lockfile is tracked and reviewed; the clone is not. Two guards follow from that
  # asymmetry. First, a destination outside `.claude/skills/` would mean a lockfile edit
  # could aim the `rm -rf` below at anything, so it is refused rather than trusted.
  case "$to" in
    .claude/skills/*/|.claude/skills/*) : ;;
    *) fail "lockfile destination $to is outside .claude/skills/" ;;
  esac
  case "$to" in
    */../*|../*|*/..) fail "lockfile destination $to contains .." ;;
  esac

  # Second, `cp -R` copies a symlink as a symlink. An upstream tree containing
  # `evil -> /home/you/.ssh` would put a live pointer to it inside a directory agents
  # read by design. Nothing we install legitimately needs one.
  if [ -n "$(find "$src" -type l -print -quit)" ]; then
    fail "$src contains a symlink; refusing to install it. Inspect it before proceeding."
  fi

  if [ "$MODE" = "check" ]; then
    [ -f "$to/SKILL.md" ] || fail "$to/SKILL.md is missing; run scripts/skill-authoring-setup.sh"
    if ! diff -r -q "$src" "$to" \
        --exclude=PROVENANCE.md --exclude=LICENSE --exclude=LICENSE.txt >/dev/null; then
      fail "$to differs from $slug/$from at ${commit:0:12}"
    fi
    printf '%s  ok  %s%s\n' "$GREEN" "$to" "$OFF"
    continue
  fi

  printf '%s==> installing %s%s\n' "$BOLD" "$to" "$OFF"
  rm -rf "$to"
  mkdir -p "$(dirname "$to")"
  cp -R "$src" "$to"

  # `$lic` sits at the clone root for the MIT repositories, outside `$src`, so the scan
  # above never saw it -- and `[ -f ]` follows a symlink while `cp` copies its target's
  # contents. A licence that is a link to something else is not a licence.
  [ ! -L "$clone/$lic" ] || fail "licence $clone/$lic is a symlink; refusing to copy it"
  if [ -f "$clone/$lic" ]; then
    cp "$clone/$lic" "$to/$(basename "$lic")"
  else
    fail "licence $clone/$lic not found; refusing to install $to without it"
  fi

  cat > "$to/PROVENANCE.md" <<EOF
# Provenance

Third-party development skill, copied verbatim. Do not edit it here: an edit would be
discarded by the next \`scripts/skill-authoring-setup.sh\` run, and it would no longer be
the upstream methodology this repository pinned.

| | |
| --- | --- |
| Upstream | $url |
| Path | \`$from\` |
| Commit | \`$commit\` |
| Licence | $slic (see \`$(basename "$lic")\` in this directory) |
| Installed by | \`scripts/skill-authoring-setup.sh\`, from \`skill-authoring-lock.json\` |

This directory is gitignored. It is development tooling; \`jev\` does not depend on it.
Why it is here, and when to use it: \`docs/development/skill-authoring.md\`.
EOF
  printf '%s  ok  %s%s\n' "$GREEN" "$to" "$OFF"
done <<< "$LOCK_INSTALLS"

if [ "$MODE" = "check" ]; then
  printf '\n%sThe pinned skill-authoring toolchain is present and unmodified.%s\n' "$GREEN" "$OFF"
  exit 0
fi

printf '\n%sDone.%s Restart Claude Code so it discovers the new skills in .claude/skills/.\n' \
  "$GREEN" "$OFF"
printf 'Read %sdocs/development/skill-authoring.md%s before writing or changing a shipped skill.\n' \
  "$BOLD" "$OFF"
printf '%sThe clones under references/ are untrusted reference material: read them, never run them.%s\n' \
  "$YELLOW" "$OFF"

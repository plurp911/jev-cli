#!/usr/bin/env bash
# Run the shipped skills' evaluation suite.
#
# Default: the instrumented Codex app-server catalog, GPT-6.1 Sol at high effort
# for both measured answers and independent rubric judges. It uses the existing
# ChatGPT login, removes API-key environment sources, and refuses API-key auth.
# Reports include token usage; subscription dollar billing is unavailable.
# This evaluates the supplied portable Skill/Read/Glob/Grep catalog, rather than
# native discovery in Claude, Cursor, or Codex. Separate native Codex trials are
# documented in docs/development/clef-skill-verification.json.
#
# Every confirmation case has a no-skill baseline and a skill arm, with its
# declared three repeats. The baseline measures the observed failure and the
# report records the delta. Errors are separate from behavioral scores.
# See docs/development/skill-authoring.md for evidence and held-out procedure.
#
# Usage (default Codex adapter):
#   scripts/skill-eval.sh                       # every training/routing case
#   scripts/skill-eval.sh --iterate             # one skill-arm development run
#   scripts/skill-eval.sh --case 'jev-*'         # exact case name or glob
#   scripts/skill-eval.sh --tag trigger          # tags select their union
#   scripts/skill-eval.sh --list                 # list cases; no model calls
#   scripts/skill-eval.sh --heldout              # final held-out set; no --iterate
#   scripts/skill-eval.sh -- --concurrency 8     # bounded parallel sessions
#
# For Codex, `--` is an optional separator: arguments reach the Python adapter.
# Model and effort are fixed explicitly to the requested GPT-6.1 Sol high;
# Claude model/effort flags and environment overrides do not affect this path.
#
# `--iterate` retains that model but runs one pass without a baseline. It helps
# check a small edit; it cannot establish a delta or repeatability. Never report
# an iterate run as confirmation. Keep held-out prompts untouched until the
# candidate is fixed, and inspect failed transcripts before changing skills.
#
# Historical Claude adapter (explicit opt-in only):
#   scripts/skill-eval.sh --list               # list the complete corpus on Codex
#   JEV_SKILL_EVAL_RUNTIME=claude scripts/skill-eval.sh --case jev-clef-behaviour-credentials -- --model claude-opus-5-5
#
# Selected skill_order cases are refused before Claude startup, including legacy
# --list on the whole corpus. Default Codex --list covers the complete corpus.
# That path uses the pinned `claude plugin eval` harness, stages a plugin copy
# under a unique target/ directory, and charges the operator's model credential.
# Its historical defaults are Claude Opus 5.5 low for measured and judge calls.
# Only on that path does everything after `--` pass through to the Claude
# harness, and JEV_SKILL_EVAL_MODEL / JEV_SKILL_EVAL_EFFORT can override defaults.
# An existing CLAUDE_CODE_EFFORT_LEVEL wins there. The legacy cheaper-model
# experiments showed different routing behavior; they are historical evidence,
# not measurements under the current GPT-6.1 Sol high policy.
#
# Do not invoke the historical adapter when the requested evaluation model is
# GPT-6.1 Sol high. No substitute model supplies evidence for that requirement.
#
set -Eeuo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

# The portable suite defaults to the instrumented Codex catalog. A historical
# Claude harness remains explicitly selectable; it is never started by default.
case "${JEV_SKILL_EVAL_RUNTIME:-codex}" in
  codex)
    CODEX_ARGUMENTS=()
    for argument in "$@"; do
      [ "$argument" = -- ] || CODEX_ARGUMENTS+=("$argument")
    done
    exec python3 scripts/skill-eval-codex.py ${CODEX_ARGUMENTS[@]+"${CODEX_ARGUMENTS[@]}"}
    ;;
  claude) ;;
  *) printf 'JEV_SKILL_EVAL_RUNTIME must be codex or claude\n' >&2; exit 2 ;;
esac

SRC_SKILLS="skills"
SRC_EVALS="evals/skills"
# One staging tree per run. A shared one was clobbered by a concurrent run: starting
# `--heldout` while a training pass was in flight re-staged the tree with only the
# held-out cases, deleting the case files and fixtures the first run was still reading.
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)-$$"
STAGE="target/skill-evals/$RUN_ID"

LIST_ONLY=0
ITERATE=0
HELDOUT=0
PASSTHROUGH=()
while [ "$#" -gt 0 ]; do
  case "$1" in
    --list) LIST_ONLY=1; shift ;;
    --iterate) ITERATE=1; shift ;;
    --heldout) HELDOUT=1; shift ;;
    --help|-h) sed -n '2,48p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    --) shift; PASSTHROUGH+=("$@"); break ;;
    *) PASSTHROUGH+=("$1"); shift ;;
  esac
done

# The model a run is measured on. Deliberately not inherited: see the header.
# A full model id, not an alias. `opus` is an alias for whatever the latest Opus is, and
# it moved from Opus 5 to Opus 5.5 while this suite was being built -- the same command
# silently started measuring a different model. A report is a measurement of one model,
# so the id is written down here and changed deliberately.
CONFIRM_MODEL="${JEV_SKILL_EVAL_MODEL:-claude-opus-5-5}"
ITERATE_MODEL="${JEV_SKILL_EVAL_ITERATE_MODEL:-$CONFIRM_MODEL}"

# The model that grades `llm` rubrics. The harness defaults to Haiku, and Haiku is too
# literal for these rubrics: it failed a response for *saying* "I left urgency alone"
# under a rubric that expressly allows stating the exclusion, and the same shape -- a
# correct, thorough answer marked wrong -- recurred across cases until it was traced to
# the judge rather than the rubric. Moving the judge off Haiku turned the measured delta
# on that case from -0.22 into +0.11, and grading was about 7% of the run's cost. A grader
# that is cheap and wrong is not cheap. It grades with the same model it measures --
# the effective one, resolved below once the overrides are known.

# Reasoning effort for the child sessions. Exported rather than passed as a flag because
# the harness's --model takes no effort suffix: `opus[low]` and `claude-opus-5[low]` are
# both rejected as unknown models. An operator who has already set this keeps it.
export CLAUDE_CODE_EFFORT_LEVEL="${CLAUDE_CODE_EFFORT_LEVEL:-${JEV_SKILL_EVAL_EFFORT:-low}}"

# Anything the caller passed after `--` wins. Checking rather than relying on
# last-flag-wins keeps this true whichever way the harness resolves duplicates.
has_flag() {
  local needle="$1" arg
  for arg in ${PASSTHROUGH[@]+"${PASSTHROUGH[@]}"}; do
    case "$arg" in "$needle"|"$needle"=*) return 0 ;; esac
  done
  return 1
}

# The value the caller passed for a flag, as `--flag value` or `--flag=value`; the
# last one if repeated. Prints nothing when the flag is absent.
flag_value() {
  local needle="$1" arg found="" take=0
  for arg in ${PASSTHROUGH[@]+"${PASSTHROUGH[@]}"}; do
    if [ "$take" = 1 ]; then found="$arg"; take=0; continue; fi
    case "$arg" in
      "$needle") take=1 ;;
      "$needle"=*) found="${arg#"$needle"=}" ;;
    esac
  done
  printf '%s' "$found"
}

# The model this run actually measures: a passed `--model` beats the per-depth default.
# The judge follows it unless it is set explicitly, so an override of the measured model
# cannot quietly leave the grading on a different one.
if [ "$ITERATE" = 1 ]; then MODEL="$ITERATE_MODEL"; else MODEL="$CONFIRM_MODEL"; fi
has_flag --model && MODEL="$(flag_value --model)"
JUDGE_MODEL="${JEV_SKILL_EVAL_JUDGE_MODEL:-$MODEL}"
has_flag --judge-model && JUDGE_MODEL="$(flag_value --judge-model)"

DEFAULTS=()
has_flag --judge-model || DEFAULTS+=(--judge-model "$JUDGE_MODEL")
has_flag --model       || DEFAULTS+=(--model "$MODEL")
if [ "$ITERATE" = 1 ]; then
  has_flag --runs      || DEFAULTS+=(--runs 1)
  has_flag --ablation  || DEFAULTS+=(--ablation none)
  printf 'iterating on %s at effort %s, graded by %s, 1 run, no baseline arm -- a signal, not a result\n' \
    "$MODEL" "$CLAUDE_CODE_EFFORT_LEVEL" "$JUDGE_MODEL" >&2
else
  printf 'measuring on %s at effort %s, graded by %s\n' \
    "$MODEL" "$CLAUDE_CODE_EFFORT_LEVEL" "$JUDGE_MODEL" >&2
fi

# Refuse selected custom graders that the historical adapter cannot implement,
# before starting Claude validation or billed sessions.
LEGACY_PREFLIGHT=()
[ "$HELDOUT" = 0 ] || LEGACY_PREFLIGHT+=(--heldout)
python3 scripts/skill-eval-codex.py --legacy-preflight \
  ${LEGACY_PREFLIGHT[@]+"${LEGACY_PREFLIGHT[@]}"} \
  ${PASSTHROUGH[@]+"${PASSTHROUGH[@]}"}

if ! command -v claude >/dev/null 2>&1; then
  printf 'claude is not on PATH; install Claude Code to run skill evals\n' >&2
  exit 127
fi
[ -d "$SRC_SKILLS" ] || { printf 'no %s directory\n' "$SRC_SKILLS" >&2; exit 1; }
[ -d "$SRC_EVALS" ]  || { printf 'no %s directory\n' "$SRC_EVALS" >&2; exit 1; }

# --- Stage. ------------------------------------------------------------------------

rm -rf "$STAGE"
mkdir -p "$STAGE/.claude-plugin" "$STAGE/skills"
cp -R "$SRC_SKILLS"/. "$STAGE/skills/"
cp -R "$SRC_EVALS"/. "$STAGE/evals/"
# Results from earlier runs live under the eval directory and would otherwise be copied
# into every later staging tree, growing without bound and putting old reports in front
# of the harness.
rm -rf "$STAGE/evals/results"

# Held-out cases are staged only when asked for, and then alone. A held-out set that
# runs alongside the training cases gets looked at while descriptions are being tuned,
# and a case that has been looked at is a training case. Run it once per final
# candidate description, at full depth -- never to iterate.
if [ "$HELDOUT" = 1 ]; then
  if [ "$ITERATE" = 1 ]; then
    printf -- '--heldout is refused under --iterate: a held-out set measured cheaply and\n' >&2
    printf 'repeatedly while tuning stops being held out\n' >&2
    exit 2
  fi
  find "$STAGE/evals" -mindepth 1 -maxdepth 1 ! -name heldout -exec rm -rf {} +
else
  rm -rf "$STAGE/evals/heldout"
fi

VERSION="$(python3 -c "
import re,sys
t=open('Cargo.toml',encoding='utf-8').read()
m=re.search(r'^version = \"([^\"]+)\"', t, re.M)
print(m.group(1) if m else '0.0.0')
")"

cat > "$STAGE/.claude-plugin/plugin.json" <<EOF
{
  "name": "jev-skills",
  "version": "$VERSION",
  "description": "Staging copy of the Agent Skills this repository ships, built by scripts/skill-eval.sh so the eval harness can load them. Not a published plugin and not a runtime component of the jev CLI.",
  "author": { "name": "jev-cli contributors" },
  "license": "MIT OR Apache-2.0"
}
EOF

claude plugin validate --strict "$STAGE" >/dev/null || {
  printf 'the staged skill plugin does not validate; fix the skills, not the staging\n' >&2
  claude plugin validate --strict "$STAGE" >&2
  exit 1
}

if [ "$LIST_ONLY" = 1 ]; then
  printf 'staged %s\n\ncases:\n' "$STAGE"
  find "$STAGE/evals" \( -name case.yaml -o -name prompt.md \) -printf '  %P\n' | sed 's#/[^/]*$##' | sort -u
  exit 0
fi

# --- Run. --------------------------------------------------------------------------
#
# --allow-tools, --allow-real-servers and --mocks off are deliberately absent. A skill
# eval needs a prompt and a transcript, not a shell or a live server. Add one
# explicitly after `--` if a case ever genuinely needs it, and say so in the pull
# request.
#
# --trust-plugin and --scaffold are honest here and only here: the plugin under test is
# this repository's own skills and its own eval cases, staged from this checkout a
# moment ago, and the scaffold scripts are committed files in this repository that do
# nothing but copy a fixture into the agent's working directory.
#
# --scaffold is not a convenience. The eval sandbox starts the agent in an empty
# working directory and refuses reads above it, and `context.scaffold_script` is the
# only mechanism that puts a file there. Without it, every case whose subject is a
# fixture runs against an empty workspace and scores the agent's inability to find the
# file rather than its reasoning -- which is exactly what the two `jev-opportunity-audit`
# fixture cases were silently doing before this flag was added.

exec claude plugin eval "$STAGE" \
  --trust-plugin \
  --scaffold \
  --no-publish \
  --max-cost-usd "${JEV_SKILL_EVAL_BUDGET_USD:-15}" \
  --output-dir "evals/skills/results/$RUN_ID" \
  ${DEFAULTS[@]+"${DEFAULTS[@]}"} \
  ${PASSTHROUGH[@]+"${PASSTHROUGH[@]}"}

#!/usr/bin/env bash
# Run the shipped skills' evaluation suite.
#
# `claude plugin eval` is the harness. It runs each case twice -- once with the skills
# loaded and once with no plugin at all -- and reports the delta. That second arm is the
# point: it is the RED baseline, measured rather than assumed, and a case whose delta is
# zero is a case the skill did not change. See docs/development/skill-authoring.md.
#
# The harness resolves skills from a plugin directory, and a plugin manifest may not
# reference a path outside its own root. Rather than put a Claude-specific manifest into
# `skills/`, which has to stay a portable Agent Skills tree, this stages a copy under
# `target/` and evaluates that. The staging tree is regenerated every run.
#
# Usage:
#   scripts/skill-eval.sh                      # every case, on the confirm model
#   scripts/skill-eval.sh --iterate            # one run, no baseline arm -- see below
#   scripts/skill-eval.sh --case 'jev-*'       # one case, or a glob
#   scripts/skill-eval.sh --tag trigger        # one tag
#   scripts/skill-eval.sh --list               # stage and list the cases, run nothing
#   scripts/skill-eval.sh --heldout            # the held-out routing set, alone; never under --iterate
#   scripts/skill-eval.sh -- --runs 5 --model claude-opus-5-5
#
# Everything after `--` is passed to `claude plugin eval` unchanged, and anything you
# pass yourself wins over the defaults this script sets.
#
# This costs money. Every run and every LLM grader is a real model call on your own
# credential, and the harness spawns a whole Claude session per run -- so a full suite
# is by far the largest thing in this repository's development budget.
#
# ## One model, two depths
#
# `--iterate` runs one pass with no baseline arm, on the same model the results are
# quoted from. With `--runs 1 --ablation none` it is roughly a sixth of a full default
# pass. Use it for the red-green loop, where you are asking "did my edit change
# anything", not "is this good".
#
# It deliberately does **not** drop to a smaller model, and that is measured. A full
# 84-case pass on Haiku scored 0.00 on every single trigger case, because a small model
# rarely reaches for a skill at all; and the `jev` skill's over-triggering on the
# plain-grep near miss reproduced only on Opus, with Haiku and Sonnet both behaving
# correctly. A cheaper model does not give a noisier version of the same signal. It gives
# a different signal, and tuning against it means tuning for a model nobody runs.
#
# **Do not report an iterate run as a result.** One run is noise-dominated, and with no
# baseline arm there is no delta.
#
# ## Why an explicit --model and effort at all
#
# Without them, each child session inherits whatever the *calling* Claude Code is
# configured with -- including a 1M-context variant and whatever reasoning effort the
# operator happens to be on. These runs need neither: they are short, single-file, and
# the judgment they exercise is the skill's, not the harness's. Inheritance also makes a
# report unreproducible, because "whatever was configured that day" is not a model.
#
# So this pins `--model claude-opus-5-5` (a full id: not the `opus` alias, which moves,
# and not `opus[1m]`) and `CLAUDE_CODE_EFFORT_LEVEL=low`.
# Measured on one identical case at one run: the same score, cost within noise, and wall
# clock halved, 58s to 29s. Effort is the smaller of the two levers -- the model tier is
# where the money is -- but it is free.
#
# Override with JEV_SKILL_EVAL_MODEL / JEV_SKILL_EVAL_EFFORT, or `-- --model ...`. If
# CLAUDE_CODE_EFFORT_LEVEL is already set in the environment, that wins and nothing here
# touches it.
set -Eeuo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

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

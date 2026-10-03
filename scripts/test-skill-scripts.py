#!/usr/bin/env python3
"""Test the deterministic helper scripts that ship inside `skills/`.

A shipped skill's script is published surface: a user installs the skill into
their own agent and that agent runs the script over their own transcript
store. It is held to the same bar as the CLI, and for the same reason -- the
failure mode is silent. An adapter that stops matching a provider's format
does not raise; it yields nothing, and the report then says the provider was
examined and no pattern was found. That is a wrong answer wearing a coverage
statement.

Fixtures live under `evals/skills/<skill>/fixtures/` next to the eval cases
that point at the same trees, so the thing the tests assert and the thing the
model is evaluated against cannot drift apart.

Dependency-free on purpose: it runs anywhere python3 does, with no pip step,
the same as every other check in `scripts/verify.sh`.
"""

from __future__ import annotations

import importlib.util
import io
import json
import sys
import unittest
from contextlib import redirect_stderr, redirect_stdout
from datetime import datetime, timezone
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "skills" / "jev-workflow-retro" / "scripts" / "transcripts.py"
FIXTURES = ROOT / "evals" / "skills" / "jev-workflow-retro" / "fixtures"
RICH = FIXTURES / "home-rich"
BARREN = FIXTURES / "home-barren"
EXPORTS = FIXTURES / "exports"

# The secrets planted in the sensitive fixture. If any of these reaches the
# output, the redaction is not doing its job, and the whole local-first privacy
# claim in the skill is false.
# Deliberately not key-shaped. A committed fixture carrying a realistic credential
# literal trips the repository's secret scanner, and the redactor's `api_key=` rule
# fires on the assignment rather than on the value, so an inert value tests it just as
# well. The value-shape patterns are covered separately, below.
PLANTED_SECRETS = (
    "SYNTHETIC-FIXTURE-VALUE-NOT-A-CREDENTIAL-0001",
    "SYNTHETIC-FIXTURE-VALUE-NOT-A-CREDENTIAL-0002",
    "hunter2",
)


def load_module():
    spec = importlib.util.spec_from_file_location("jev_retro_transcripts", SCRIPT)
    if spec is None or spec.loader is None:  # pragma: no cover - import plumbing
        raise SystemExit(f"cannot load {SCRIPT}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


transcripts = load_module()


class FixtureDatetime(datetime):
    """Keep dated transcript fixtures inside a reproducible default window."""

    @classmethod
    def now(cls, tz=None):
        moment = cls(2026, 9, 20, 12, tzinfo=timezone.utc)
        return moment.astimezone(tz) if tz is not None else moment.replace(tzinfo=None)


def run(*argv: str) -> tuple[str, str]:
    """Invoke the script's own entry point and capture both streams."""
    out, err = io.StringIO(), io.StringIO()
    # Fixture timestamps are immutable. A real clock made three normalization tests
    # lose their September sidechain/spawn events as the 30-day window advanced.
    # Pin the clock, preserving the default window and every explicit filter.
    with redirect_stdout(out), redirect_stderr(err), patch.object(transcripts, "datetime", FixtureDatetime):
        transcripts.main(list(argv))
    return out.getvalue(), err.getvalue()


def events(*argv: str) -> list[dict]:
    out, _ = run("events", *argv)
    return [json.loads(line) for line in out.splitlines() if line.strip()]


def summary(*argv: str) -> dict:
    out, _ = run("summary", *argv)
    return json.loads(out)


def discover(*argv: str) -> dict:
    out, _ = run("discover", *argv)
    return json.loads(out)


class Discovery(unittest.TestCase):
    def test_finds_both_providers_in_the_rich_fixture(self):
        document = discover("--home", str(RICH))
        found = {entry["provider"]: entry for entry in document["found"]}
        self.assertEqual({"claude-code", "codex", "gemini-cli"}, set(found))
        # Ten sessions. There are eleven files -- one is a subagent sidechain, which
        # carries its parent's id -- and this assertion used to say eleven, pinning the
        # defect in place: discovery counted files as sessions, 1,034 on a real store of
        # 399.
        self.assertEqual(10, found["claude-code"]["sessions"])
        self.assertEqual(4, found["codex"]["sessions"])
        self.assertEqual(2, found["gemini-cli"]["sessions"])

    def test_reports_absent_providers_rather_than_omitting_them(self):
        document = discover("--home", str(RICH))
        absent = {entry["provider"] for entry in document["absent"]}
        # A provider with no store must be named as absent. Saying nothing
        # about it reads as "examined and found nothing", which is a different
        # and much stronger claim than "not installed".
        self.assertIn("cline", absent)
        self.assertNotIn("gemini-cli", absent)  # it has sessions in this fixture now

    def test_names_the_stores_it_deliberately_does_not_parse(self):
        document = discover("--home", str(RICH))
        not_parsed = {entry["provider"] for entry in document["not_parsed"]}
        for provider in ("cursor", "copilot-chat", "grok-cli", "opencode", "pi"):
            self.assertIn(provider, not_parsed)

    def test_an_empty_home_is_not_an_error(self):
        document = discover("--home", str(FIXTURES / "does-not-exist"))
        self.assertEqual([], document["found"])
        self.assertEqual(0, document["problem_count"])


class Normalisation(unittest.TestCase):
    def test_every_event_carries_the_schema_contract(self):
        for event in events("--home", str(RICH)):
            self.assertEqual(transcripts.EVENT_SCHEMA, event["schema"])
            for key in ("provider", "session", "kind", "seq"):
                self.assertIn(key, event)
            self.assertIn(event["kind"], transcripts.KINDS)

    def test_claude_code_yields_each_kind(self):
        kinds = {e["kind"] for e in events("--home", str(RICH), "--provider", "claude-code")}
        self.assertEqual(
            {"session", "prompt", "assistant", "tool_call", "tool_result", "subagent", "skill"},
            kinds,
        )

    def test_subagent_spawns_are_their_own_kind_with_the_agent_type(self):
        spawns = [e for e in events("--home", str(RICH), "--kind", "subagent")]
        types = {e["agent_type"] for e in spawns}
        # A routing decision is the pattern this skill exists to find. Burying
        # it in an `Agent` row of a tool histogram loses the thing that makes
        # it a decision: which specialist was chosen.
        self.assertIn("docs-writer", types)
        self.assertIn("security-reviewer", types)

    def test_skill_invocations_are_their_own_kind(self):
        names = {e["skill"] for e in events("--home", str(RICH), "--kind", "skill")}
        self.assertIn("security-review", names)
        self.assertIn("technical-writing", names)

    def test_injected_user_turns_are_not_counted_as_prompts(self):
        prompts = events("--home", str(RICH), "--kind", "prompt")
        # Every fixture prompt is a sentence a person typed. A tool result fed
        # back as a `user` record must not appear here, or every frequency the
        # report quotes is inflated by the number of tool calls.
        for event in prompts:
            self.assertNotIn("tool_use_id", json.dumps(event))
        self.assertTrue(all(event.get("text") for event in prompts))

    def test_codex_resolves_its_project_from_inside_the_file(self):
        document = discover("--home", str(RICH), "--project", "helio")
        found = {entry["provider"] for entry in document["found"]}
        # Codex stores the working directory in the file, not the path. When
        # this regressed, `--project` silently discarded every Codex session
        # and the report still claimed Codex was examined.
        self.assertIn("codex", found)

    def test_codex_tool_calls_and_usage_are_normalised(self):
        found = events("--home", str(RICH), "--provider", "codex")
        self.assertTrue(any(e["kind"] == "tool_call" and e["tool"] == "shell" for e in found))
        self.assertTrue(
            any(isinstance(e.get("usage"), dict) and e["usage"].get("input_tokens") for e in found)
        )


class ProviderFormats(unittest.TestCase):
    """The formats that were silently losing data on real machines.

    Each of these was found by running the adapters against a real store and comparing
    the counts against the files. None of them raised; they simply returned less, which
    is the failure mode this whole module exists to make loud.
    """

    def test_codex_reads_the_current_conversation_shape(self):
        # Newer Codex builds stopped emitting `event_msg/user_message` and carry the
        # conversation as `response_item/message` instead. Handling only the old pair
        # dropped every prompt and every assistant turn in a current session while tool
        # calls still came through, so a session read as pure tool use.
        found = events("--home", str(RICH), "--provider", "codex")
        texts = [e.get("text", "") for e in found if e["kind"] == "prompt"]
        self.assertTrue(any("flaky-looking failures" in t for t in texts), texts)
        self.assertTrue(any(e["kind"] == "assistant" for e in found))

    def test_codex_reports_a_pre_envelope_rollout_instead_of_yielding_nothing(self):
        document = summary("--home", str(RICH), "--provider", "codex")
        reasons = " ".join(problem["reason"] for problem in document["problems"])
        self.assertIn("no {type, payload} envelope", reasons)

    def test_codex_subagent_activity_is_a_subagent_event(self):
        kinds = {(e["kind"], e.get("agent_type"))
                 for e in events("--home", str(RICH), "--provider", "codex")}
        self.assertIn(("subagent", "test-triager"), kinds)

    def test_gemini_content_parts_without_a_type_are_still_text(self):
        # Gemini's native part is `{"text": …}` with no `type` key. Requiring
        # `type == "text"` dropped every prompt on disk and reported no problem.
        texts = [e.get("text", "")
                 for e in events("--home", str(RICH), "--provider", "gemini-cli")
                 if e["kind"] == "prompt"]
        self.assertTrue(any("search hits are on the billing path" in t for t in texts), texts)

    def test_gemini_whole_document_sessions_are_read_as_documents(self):
        # A `.json` file in the same directory is one pretty-printed document, not
        # JSONL. Reading it line by line produced thousands of "unparsable line"
        # problems and no events, while discovery still counted it as a session.
        texts = [e.get("text", "")
                 for e in events("--home", str(RICH), "--provider", "gemini-cli")
                 if e["kind"] == "prompt"]
        self.assertTrue(any("export worker" in t for t in texts), texts)
        document = summary("--home", str(RICH), "--provider", "gemini-cli")
        self.assertNotIn("unparsable", " ".join(p["reason"] for p in document["problems"]))

    def test_claude_code_subagent_sidechains_are_read(self):
        # A subagent's own turns live only in its sidechain file, three levels deep. A
        # two-level glob meant the delegated half of every session was invisible, and
        # the `isSidechain` handling was unreachable.
        found = events("--home", str(RICH), "--provider", "claude-code")
        side = [e for e in found if e.get("sidechain")]
        self.assertTrue(side, "no sidechain events were read")
        self.assertTrue(
            any("timing or real" in (e.get("text") or "") for e in side),
            [e.get("text") for e in side],
        )

    def test_a_subagent_file_rolls_into_its_parent_session(self):
        found = events("--home", str(RICH), "--provider", "claude-code")
        sessions = {e["session"] for e in found if e.get("sidechain")}
        parents = {e["session"] for e in found if not e.get("sidechain")}
        self.assertTrue(sessions <= parents, sessions - parents)


class Privacy(unittest.TestCase):
    def test_no_planted_secret_survives_normalisation(self):
        blob = "\n".join(json.dumps(e) for e in events("--home", str(RICH)))
        for secret in PLANTED_SECRETS:
            self.assertNotIn(secret, blob, f"{secret!r} reached the output")

    def test_no_planted_secret_survives_the_summary(self):
        blob = json.dumps(summary("--home", str(RICH)))
        for secret in PLANTED_SECRETS:
            self.assertNotIn(secret, blob)

    def test_tool_results_are_metadata_not_content(self):
        for event in events("--home", str(RICH), "--kind", "tool_result"):
            # The output of a tool is where a credential, a customer record or
            # a file's contents actually lands. Its size and whether it failed
            # are enough to see a workflow shape; its body is not needed and
            # is the single largest disclosure in a transcript store.
            self.assertNotIn("text", event)
            self.assertIn("bytes", event)

    def test_text_is_clipped_to_the_requested_budget(self):
        for event in events("--home", str(RICH), "--max-chars", "40"):
            self.assertLessEqual(len(event.get("text") or ""), 40)

    def test_tool_arguments_are_digested_not_reproduced(self):
        for event in events("--home", str(RICH), "--kind", "tool_call", "--arg-chars", "25"):
            for key, value in (event.get("args") or {}).items():
                if isinstance(value, str) and not key.startswith("_"):
                    self.assertLessEqual(len(value), 26)  # 25 plus the ellipsis

    def test_redaction_covers_the_common_credential_shapes(self):
        cases = [
            "ghp_" + "A" * 24,
            "AKIAIOSFODNN7EXAMPLE",  # the AWS documentation example, not a key
            "Authorization: Bearer abcdefghijklmnop",
            "api_key = 'abcdefghijklmnop'",
            "-----BEGIN " + "RSA PRIVATE KEY-----\nMIIE\n-----END " + "RSA PRIVATE KEY-----",  # split so no scanner reads it as a key
        ]
        for case in cases:
            self.assertIn("<redacted>", transcripts.redact(case), case)

    # Built at run time from fragments. A contiguous vendor-key-shaped literal in a
    # committed file trips the repository's secret scanner, and the point here is the
    # *shape*, not any particular value.
    _STRIPE_SHAPED = "sk" + "_live_" + "NOTAREALKEY" + "0" * 20

    def test_redaction_catches_the_shapes_that_dominate_a_transcript(self):
        # Every one of these reached stdout unredacted at some point. The first four
        # failed because the keyword pattern wrapped its keyword in `\b`, and `_` is a
        # word character, so the boundary never fired inside an identifier -- which is
        # the shape almost every real credential in a transcript actually has.
        cases = [
            "AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMIK7MDENGbPxRfiCYEXAMPLEKEY",
            "STRIPE_SECRET=" + self._STRIPE_SHAPED,
            "DB_PASSWORD=hunter2ExtraLongEnough",
            "HELIO_API_KEY=some-value-long-enough-to-matter",
            # Stripe separates with an underscore; requiring a literal `sk-` let every
            # real Stripe key through.
            "stripe_key=" + self._STRIPE_SHAPED,
            # A credential in a URL authority has no keyword anywhere near it.
            "DATABASE_URL=postgres://helio:hunter2@db.internal:5432/helio",
        ]
        for case in cases:
            self.assertIn("<redacted>", transcripts.redact(case), case)

    def test_a_credential_under_a_sensitive_argument_key_is_redacted(self):
        # `safe_args` splits a tool call's arguments into separate key/value entries,
        # which breaks the adjacency the keyword patterns depend on. A value under a key
        # literally named `password` was emitted in full.
        args = transcripts.safe_args(
            {"password": "hunter2ExtraLongEnough", "command": "echo hi",
             "api_key": self._STRIPE_SHAPED},
            200,
        )
        self.assertEqual("<redacted>", args["password"])
        self.assertEqual("<redacted>", args["api_key"])
        self.assertEqual("echo hi", args["command"])

    def test_redaction_does_not_eat_ordinary_prose(self):
        prose = "The digest job drops tickets updated in the last 60 seconds."
        self.assertEqual(prose, transcripts.redact(prose))

    def test_redaction_catches_serialised_flag_and_vendor_prefixed_shapes(self):
        # Each of these passed the redactor untouched when an independent review tried
        # it. Values are assembled from fragments for the same reason as `_STRIPE_SHAPED`.
        value = "not-a-real-value-0003"
        cases = {
            # A closing quote sits between the keyword and the colon.
            '{"api_key": "' + value + '"}': value,
            "{'password': '" + value + "'}": value,
            '{"client_secret": "has spaces ' + value + '"}': value,
            "mysql --password " + value: value,
            "gh auth login --token " + value: value,
            "curl -u deploy:" + value + " https://example.invalid": value,
            "github" + "_pat_" + "A" * 30: "A" * 30,
            "gl" + "pat-" + "B" * 20: "B" * 20,
            "xa" + "pp-1-" + "C" * 16: "C" * 16,
            "np" + "m_" + "D" * 36: "D" * 36,
            "h" + "f_" + "E" * 34: "E" * 34,
            "AS" + "IA" + "EXAMPLE0EXAMPLE0": "EXAMPLE0EXAMPLE0",
            "-----BEGIN " + "PGP PRIVATE KEY BLOCK-----\nlQOYBF\n-----END "
            + "PGP PRIVATE KEY BLOCK-----": "lQOYBF",
        }
        for case, body in cases.items():
            out = transcripts.redact(case)
            self.assertIn("<redacted>", out, case)
            self.assertNotIn(body, out, case)

    def test_a_private_key_with_no_end_line_is_redacted_to_the_end_of_the_field(self):
        # What a key cut off by the scan window, or pasted half-way, looks like.
        text, _ = transcripts.clip(
            "here it is -----BEGIN " + "OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXkt\nAAAA", 600
        )
        self.assertEqual("here it is <redacted>", text)

    def test_a_redacted_flag_keeps_its_name(self):
        # Which flag was passed is workflow signal; only its value is the secret.
        self.assertEqual("psql --password <redacted>",
                         transcripts.redact("psql --password not-a-real-value"))

    def test_the_wider_patterns_leave_similar_prose_alone(self):
        for text in (
            "Please enter your password to continue.",
            "the token: yes",
            "docker login --password-stdin",
            "pg_dump --password-file /run/secrets/db-password",
            "docker run -u 1000:1000 image",
            "git push -u origin main",
            "npm_config_cache is set",
        ):
            self.assertEqual(text, transcripts.redact(text))


class Bounds(unittest.TestCase):
    def test_an_enormous_field_does_not_scan_the_whole_thing(self):
        # `clip` collapsed whitespace and ran every pattern over the entire field before
        # truncating, so one 100 MB line meant gigabytes of small string objects for at
        # most `limit` characters of output.
        huge = "word " * 2_000_000
        text, truncated = transcripts.clip(huge, 50)
        self.assertEqual(50, len(text))
        self.assertTrue(truncated)

    def test_a_symlink_out_of_the_store_is_not_followed(self):
        # `Path.glob` follows symlinks. A link planted inside a transcript store is how
        # a hostile earlier session gets this skill to read, and report, a file the user
        # never named. AGENTS.md §4.
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp) / "home"
            outside = Path(tmp) / "outside"
            outside.mkdir()
            secret = outside / "elsewhere.jsonl"
            secret.write_text(
                '{"type":"user","uuid":"u1","sessionId":"s","cwd":"/x","version":"1",'
                '"timestamp":"2026-09-01T00:00:00Z","message":{"role":"user",'
                '"content":"content from outside the store"}}\n'
            )
            project = home / ".claude" / "projects" / "-x"
            project.mkdir(parents=True)
            (project / "linked.jsonl").symlink_to(secret)

            document = discover("--home", str(home))
            reasons = " ".join(p["reason"] for p in document["problems"])
            self.assertIn("resolves outside the store", reasons)
            self.assertEqual([], document["found"])

            blob = "\n".join(json.dumps(e) for e in events("--home", str(home)))
            self.assertNotIn("content from outside the store", blob)


class Resilience(unittest.TestCase):
    def test_a_corrupt_session_is_reported_and_stepped_over(self):
        document = summary("--home", str(RICH))
        reasons = " ".join(problem["reason"] for problem in document["problems"])
        self.assertIn("unparsable", reasons)
        # The readable records in the same file still have to come through:
        # skipping the file would lose a session, and failing would lose the
        # whole run over one truncated line, which is the normal state of a
        # session that is still open.
        self.assertGreater(document["totals"]["sessions"], 10)

    def test_an_unknown_provider_is_a_problem_not_a_crash(self):
        with self.assertRaises(SystemExit):
            run("discover", "--provider", "nonesuch")

    def test_garbage_in_a_session_file_yields_no_events_and_no_exception(self):
        junk = FIXTURES / "junk" / ".claude" / "projects" / "-x"
        junk.mkdir(parents=True, exist_ok=True)
        path = junk / "deadbeef-0000-4000-8000-000000000000.jsonl"
        path.write_text("\x00\x01 not json\n[]\n{}\n")
        try:
            document = summary("--home", str(junk.parents[2]))
            self.assertEqual(0, document["events_by_kind"].get("prompt", 0))
        finally:
            path.unlink()


class Counting(unittest.TestCase):
    def test_session_total_is_sessions_not_session_records(self):
        document = summary("--home", str(RICH))
        # These two were once the same key, and the per-kind count silently
        # replaced the session total with the number of session-open records.
        self.assertEqual(15, document["totals"]["sessions"])
        self.assertGreater(document["events_by_kind"]["session"], 15)

    def test_the_barren_fixture_has_no_routing_or_skill_signal(self):
        document = summary("--home", str(BARREN))
        self.assertEqual(3, document["totals"]["sessions"])
        self.assertEqual([], document["subagents"])
        self.assertEqual([], document["skills"])

    def test_max_sessions_caps_per_provider_and_says_it_capped(self):
        # Per provider, not pooled. Pooled, a provider used less recently than another
        # vanishes from the run entirely and `discover` then reports it as having no
        # sessions in the window -- a claim about the user's history rather than about
        # this run's budget.
        # The fixture has 10 Claude Code, 3 Codex and 2 Gemini CLI sessions, so two per
        # provider keeps six. Pooled, it would keep two in total.
        document = summary("--home", str(RICH), "--max-sessions", "2")
        self.assertEqual(6, document["totals"]["sessions"])
        self.assertEqual(
            ["claude-code", "codex", "gemini-cli"], document["totals"]["providers"]
        )
        capped = {p["provider"] for p in document["problems"] if "kept the 2" in p["reason"]}
        # Gemini CLI has exactly two, so it is at the cap but not over it.
        self.assertEqual({"claude-code", "codex"}, capped)

    def test_a_provider_is_never_reported_absent_because_another_filled_the_cap(self):
        document = discover("--home", str(RICH), "--max-sessions", "1")
        found = {entry["provider"] for entry in document["found"]}
        absent = {entry["provider"] for entry in document["absent"]}
        self.assertIn("codex", found)
        self.assertNotIn("codex", absent)

    def test_tool_pairs_count_adjacent_calls_within_a_session(self):
        document = summary("--home", str(RICH))
        pairs = {tuple(entry["pair"]): entry["count"] for entry in document["tool_pairs"]}
        self.assertIn(("Edit", "Bash"), pairs)
        self.assertGreater(pairs[("Edit", "Bash")], 1)


class GenericExport(unittest.TestCase):
    def test_markdown_speaker_headings_become_turns(self):
        found = events("--input", str(EXPORTS / "grok-session.md"))
        kinds = [e["kind"] for e in found]
        self.assertEqual(3, kinds.count("prompt"))
        self.assertEqual(3, kinds.count("assistant"))

    def test_a_prose_sentence_is_not_a_turn_boundary(self):
        path = FIXTURES / "junk-export.md"
        path.write_text("## User\n\nAssistant: please look at this. I asked the assistant already.\n")
        try:
            found = events("--input", str(path))
            self.assertEqual(1, sum(1 for e in found if e["kind"] == "prompt"))
        finally:
            path.unlink()

    def test_jsonl_export_maps_roles_and_tool_calls(self):
        found = events("--input", str(EXPORTS / "partner-triage.jsonl"))
        kinds = [e["kind"] for e in found]
        self.assertIn("prompt", kinds)
        self.assertIn("assistant", kinds)
        self.assertIn("tool_call", kinds)

    def test_an_unclassifiable_record_is_unknown_not_guessed(self):
        path = FIXTURES / "junk-export.jsonl"
        path.write_text('{"speaker":"nobody","body":"something happened"}\n')
        try:
            found = events("--input", str(path))
            self.assertTrue(any(e["kind"] == "unknown" for e in found))
        finally:
            path.unlink()


class Portability(unittest.TestCase):
    def test_the_script_declares_no_third_party_import(self):
        source = SCRIPT.read_text(encoding="utf-8")
        forbidden = ("import requests", "import yaml", "import numpy", "from requests")
        for needle in forbidden:
            self.assertNotIn(needle, source)

    # The no-network check is `NoNetworkNoProcess`, below: an AST import allowlist.


# --------------------------------------------------------------------------
# Real-format regressions.
#
# Every case below reproduces a record shape found on a real machine by an
# independent review, where the adapter produced a wrong number without raising.
# Each is built inline so the test shows exactly the shape it pins.
# --------------------------------------------------------------------------

import tempfile  # noqa: E402 -- kept beside the tests that need it


def _jsonl(path: Path, records: list) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("".join(
        (r if isinstance(r, str) else json.dumps(r)) + "\n" for r in records
    ), encoding="utf-8")
    return path


def _codex(home: Path, name: str, records: list) -> Path:
    return _jsonl(home / ".codex" / "sessions" / "2026" / "09" / "20" / name, records)


def _rec(kind: str, payload: dict, ts: str = "2026-09-20T10:00:00Z") -> dict:
    return {"timestamp": ts, "type": kind, "payload": payload}


def _codex_meta(sid: str, **extra) -> dict:
    return _rec("session_meta", {"id": sid, "cwd": "/w/proj", "cli_version": "0.155.1", **extra})


UUID_A = "019a0000-0000-7000-8000-00000000000a"
UUID_B = "019a0000-0000-7000-8000-00000000000b"
NOW_TS = "2099-01-01T00:00:00Z"  # far enough ahead to sit inside any --days window


class CodexRealFormats(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.home = Path(self._tmp.name)

    def tearDown(self):
        self._tmp.cleanup()

    def ev(self, *extra: str) -> list[dict]:
        return events("--home", str(self.home), "--provider", "codex", "--days", "36500", *extra)

    def test_token_count_is_a_running_total_so_only_the_last_one_counts(self):
        # Up to 1,688 of these in one real file. Summing them overcounted ~26x.
        _codex(self.home, f"rollout-2026-09-20T10-00-00-{UUID_A}.jsonl", [
            _codex_meta(UUID_A),
            *[_rec("event_msg", {"type": "token_count", "info": {"total_token_usage": {
                "input_tokens": 1000 * i, "output_tokens": 10 * i}}}) for i in (1, 2, 3)],
        ])
        document = summary("--home", str(self.home), "--provider", "codex", "--days", "36500")
        self.assertEqual({"codex": {"input_tokens": 3000, "output_tokens": 30}}, document["usage"])

    def test_a_turn_recorded_twice_is_one_prompt(self):
        # Every version on disk records a typed turn both ways.
        _codex(self.home, f"rollout-2026-09-20T10-00-00-{UUID_A}.jsonl", [
            _codex_meta(UUID_A),
            _rec("turn_context", {"cwd": "/w/proj", "model": "gpt-x"}),
            _rec("event_msg", {"type": "user_message", "message": "triage the failing tests"}),
            _rec("response_item", {"type": "message", "role": "user",
                                   "content": [{"type": "input_text", "text": "triage the failing tests"}]}),
            _rec("turn_context", {"cwd": "/w/proj", "model": "gpt-x"}, ts="2026-09-20T10:05:00Z"),
            _rec("event_msg", {"type": "user_message", "message": "triage the failing tests"},
                 ts="2026-09-20T10:05:00Z"),
        ])
        prompts = [e for e in self.ev() if e["kind"] == "prompt"]
        # Once per turn: the duplicate within the first turn is dropped, the genuine
        # repeat in the second turn is kept.
        self.assertEqual(2, len(prompts))

    def test_injected_wrapper_turns_are_not_prompts(self):
        _codex(self.home, f"rollout-2026-09-20T10-00-00-{UUID_A}.jsonl", [
            _codex_meta(UUID_A),
            *[_rec("response_item", {"type": "message", "role": "user", "content": [
                {"type": "input_text", "text": f"<{tag}>\n<x>1</x>\n</{tag}>"}]})
              for tag in ("environment_context", "subagent_notification",
                          "recommended_plugins", "turn_aborted")],
        ])
        self.assertEqual([], [e for e in self.ev() if e["kind"] == "prompt"])

    def test_a_spawn_and_its_started_activity_are_one_subagent(self):
        # On disk the two appear one for one; counting both doubled every subagent.
        _codex(self.home, f"rollout-2026-09-20T10-00-00-{UUID_A}.jsonl", [
            _codex_meta(UUID_A),
            _rec("response_item", {"type": "function_call", "namespace": "collaboration",
                                   "name": "spawn_agent", "call_id": "c1",
                                   "arguments": json.dumps({"task_name": "review auth", "message": "…"})}),
            _rec("event_msg", {"type": "item_completed", "item": {
                "type": "SubAgentActivity", "kind": "started", "agent_path": "/root/reviewer",
                "agent_thread_id": "t", "id": "i1"}}),
        ])
        spawns = [e for e in self.ev() if e["kind"] == "subagent"]
        self.assertEqual(1, len(spawns))
        self.assertEqual("collaboration.spawn_agent", spawns[0]["tool"])

    def test_started_activity_alone_still_counts_and_reads_agent_path(self):
        # The real payload names an `agent_path`; there is no `agent` field.
        _codex(self.home, f"rollout-2026-09-20T10-00-00-{UUID_A}.jsonl", [
            _codex_meta(UUID_A),
            _rec("event_msg", {"type": "sub_agent_activity", "kind": "started",
                               "agent_path": "/root/test-triager", "agent_thread_id": "t",
                               "event_id": "e", "occurred_at_ms": 1}),
            _rec("event_msg", {"type": "sub_agent_activity", "kind": "interacted",
                               "agent_path": "/root/test-triager", "agent_thread_id": "t",
                               "event_id": "e2", "occurred_at_ms": 2}),
        ])
        spawns = [e for e in self.ev() if e["kind"] == "subagent"]
        self.assertEqual(["test-triager"], [e["agent_type"] for e in spawns])

    def test_mcp_calls_are_tool_calls(self):
        # MCP calls have no `function_call` at all; they were invisible.
        _codex(self.home, f"rollout-2026-09-20T10-00-00-{UUID_A}.jsonl", [
            _codex_meta(UUID_A),
            _rec("event_msg", {"type": "item_completed", "item": {
                "type": "McpToolCall", "id": "m1", "server": "gitea", "tool": "list_commits",
                "status": "completed", "arguments": {"repo": "x"}}}),
            _rec("event_msg", {"type": "mcp_tool_call_end", "call_id": "m2", "invocation": {
                "server": "gitea", "tool": "get_commit", "arguments": {}}, "result": {"Err": "x"}}),
        ])
        calls = {e["tool"]: e for e in self.ev() if e["kind"] == "tool_call"}
        self.assertEqual({"mcp__gitea__list_commits", "mcp__gitea__get_commit"}, set(calls))
        self.assertTrue(calls["mcp__gitea__list_commits"]["ok"])
        self.assertFalse(calls["mcp__gitea__get_commit"]["ok"])

    def test_namespaced_tools_keep_their_namespace(self):
        _codex(self.home, f"rollout-2026-09-20T10-00-00-{UUID_A}.jsonl", [
            _codex_meta(UUID_A),
            _rec("response_item", {"type": "function_call", "namespace": "clock",
                                   "name": "sleep", "call_id": "c", "arguments": "{}"}),
        ])
        self.assertEqual(["clock.sleep"], [e["tool"] for e in self.ev() if e["kind"] == "tool_call"])

    def test_a_forked_rollout_keeps_its_own_session_id(self):
        # The parent's header is replayed on line 2 of a fork.
        _codex(self.home, f"rollout-2026-09-20T10-00-00-{UUID_A}.jsonl", [
            _codex_meta(UUID_A, forked_from_id=UUID_B),
            _codex_meta(UUID_B),
            _rec("event_msg", {"type": "user_message", "message": "carry on"}),
        ])
        self.assertEqual({UUID_A}, {e["session"] for e in self.ev()})

    def test_a_subagent_rollout_rolls_into_its_parent_and_its_turns_are_not_prompts(self):
        _codex(self.home, f"rollout-2026-09-20T10-00-00-{UUID_A}.jsonl", [
            _codex_meta(UUID_A, source={"subagent": {"thread_spawn": {"parent_thread_id": UUID_B}}}),
            _rec("event_msg", {"type": "user_message", "message": "task written by the parent"}),
            _rec("event_msg", {"type": "agent_message", "message": "done"}),
        ])
        found = self.ev()
        self.assertEqual({UUID_B}, {e["session"] for e in found})
        self.assertEqual([], [e for e in found if e["kind"] == "prompt"])
        self.assertTrue(any(e["kind"] == "assistant" for e in found))

    def test_the_model_is_read_from_the_turn_context(self):
        _codex(self.home, f"rollout-2026-09-20T10-00-00-{UUID_A}.jsonl", [
            _codex_meta(UUID_A),
            _rec("turn_context", {"cwd": "/w/proj", "model": "gpt-x"}),
            _rec("event_msg", {"type": "agent_message", "message": "ok"}),
        ])
        self.assertEqual(["gpt-x"], [e.get("model") for e in self.ev() if e["kind"] == "assistant"])

    def test_the_session_id_from_a_filename_is_the_whole_uuid(self):
        self.assertEqual(UUID_A, transcripts.uuid_tail(f"rollout-2026-09-20T10-00-00-{UUID_A}"))


class ClaudeCodeRealFormats(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.home = Path(self._tmp.name)
        self.path = self.home / ".claude" / "projects" / "-w-proj" / "s1.jsonl"

    def tearDown(self):
        self._tmp.cleanup()

    def user(self, content, **extra):
        return {"type": "user", "uuid": extra.pop("uuid"), "sessionId": "s1", "cwd": "/w/proj",
                "version": "2.1.280", "timestamp": NOW_TS,
                "message": {"role": "user", "content": content}, **extra}

    def ev(self):
        return events("--home", str(self.home), "--provider", "claude-code", "--days", "36500")

    def test_turns_from_something_other_than_a_person_are_not_prompts(self):
        # 40% of the prompts in a real week were background-task notifications.
        _jsonl(self.path, [
            self.user("a real question", uuid="u1", origin={"kind": "human"}),
            self.user("<task-notification>done</task-notification>", uuid="u2",
                      origin={"kind": "task-notification"}),
            self.user("hello from a peer", uuid="u3", origin={"kind": "peer"}),
            self.user("an older record with no origin at all", uuid="u4"),
        ])
        texts = [e["text"] for e in self.ev() if e["kind"] == "prompt"]
        self.assertEqual(["a real question", "an older record with no origin at all"], texts)

    def test_a_slash_command_is_a_skill_choice(self):
        _jsonl(self.path, [self.user(
            "<command-name>/security-review</command-name><command-args>pr 12</command-args>",
            uuid="u1")])
        found = self.ev()
        skills = [e for e in found if e["kind"] == "skill"]
        self.assertEqual(["security-review"], [e["skill"] for e in skills])
        self.assertEqual([], [e for e in found if e["kind"] == "prompt"])

    def test_pasted_content_is_still_the_persons_prompt(self):
        _jsonl(self.path, [self.user(
            'look at this <pasted_content id="1">the log they pasted</pasted_content id="1">',
            uuid="u1")])
        texts = [e["text"] for e in self.ev() if e["kind"] == "prompt"]
        self.assertEqual(1, len(texts))
        self.assertIn("the log they pasted", texts[0])

    def test_the_session_open_event_carries_cwd_when_the_first_line_is_bookkeeping(self):
        _jsonl(self.path, [
            {"type": "queue-operation", "sessionId": "s1", "operation": "enqueue"},
            self.user("go", uuid="u1"),
        ])
        opened = [e for e in self.ev() if e["kind"] == "session"][0]
        self.assertEqual("/w/proj", opened.get("cwd"))
        self.assertEqual("2.1.280", opened.get("agent"))


class CursorAgent(unittest.TestCase):
    def test_the_nested_anthropic_message_shape_is_read(self):
        # Every turn of every real Cursor agent transcript came out as "content".
        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp)
            _jsonl(home / ".cursor" / "projects" / "p" / "agent-transcripts" / "u1" / "t.jsonl", [
                {"role": "user", "message": {"content": [{"type": "text", "text": "fix the build"}]}},
                {"role": "assistant", "message": {"content": [
                    {"type": "text", "text": "reading the config"},
                    {"type": "tool_use", "name": "Read", "input": {"path": "a"}}]}},
                {"role": "assistant", "message": {"content": [
                    {"type": "tool_use", "name": "Shell", "input": {"command": "make"}}]}},
            ])
            found = events("--home", str(home), "--provider", "cursor-agent", "--days", "36500")
            self.assertEqual(["fix the build"], [e["text"] for e in found if e["kind"] == "prompt"])
            self.assertEqual(["reading the config"],
                             [e["text"] for e in found if e["kind"] == "assistant"])
            self.assertEqual(["Read", "Shell"], [e["tool"] for e in found if e["kind"] == "tool_call"])
            self.assertNotIn("content", [e.get("text") for e in found])


class HostileFiles(unittest.TestCase):
    """Everything unreadable has to appear in `problems`. Nothing may crash."""

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.home = Path(self._tmp.name)
        self.proj = self.home / ".claude" / "projects" / "-w"

    def tearDown(self):
        self._tmp.cleanup()

    def reasons(self, *argv):
        document = summary("--home", str(self.home), "--days", "36500", *argv)
        return " | ".join(p["reason"] for p in document["problems"]), document

    def test_a_byte_order_mark_does_not_lose_the_first_line(self):
        path = self.proj / "s1.jsonl"
        path.parent.mkdir(parents=True)
        path.write_bytes(b"\xef\xbb\xbf" + json.dumps({
            "type": "user", "uuid": "u1", "sessionId": "s1", "cwd": "/w", "timestamp": NOW_TS,
            "message": {"role": "user", "content": "first line"}}).encode() + b"\n")
        texts = [e.get("text") for e in events("--home", str(self.home), "--days", "36500")]
        self.assertIn("first line", texts)

    def test_empty_files_non_object_lines_and_directories_are_reported(self):
        _jsonl(self.proj / "empty.jsonl", [])
        _jsonl(self.proj / "scalars.jsonl", ["[1,2,3]", '"str"', "42"])
        (self.proj / "dir.jsonl").mkdir(parents=True)
        reasons, _ = self.reasons()
        self.assertIn("no records", reasons)
        self.assertIn("3 unparsable", reasons)
        self.assertIn("not a regular file", reasons)

    def test_non_string_names_do_not_crash_the_summary(self):
        _jsonl(self.proj / "s1.jsonl", [
            {"type": "assistant", "uuid": "a1", "sessionId": "s1", "cwd": {"not": "a string"},
             "timestamp": NOW_TS, "requestId": "r",
             "message": {"role": "assistant", "model": "m", "usage": {"output_tokens": "lots"},
                         "content": [{"type": "tool_use", "id": "t", "name": {"odd": 1},
                                      "input": {"subagent_type": ["x"]}}]}},
        ])
        _, document = self.reasons()
        self.assertEqual(1, document["totals"]["sessions"])

    def test_deep_nesting_in_a_codex_header_is_reported_not_raised(self):
        _codex(self.home, f"rollout-2026-09-20T10-00-00-{UUID_A}.jsonl",
               ["[" * 200_000 + "]" * 200_000])
        reasons, _ = self.reasons("--provider", "codex")
        self.assertIn("unparsable", reasons)


class UsageIsNeverPooled(unittest.TestCase):
    def test_usage_is_reported_per_provider(self):
        # Claude Code records output tokens per request but no comparable input total;
        # pooling put Codex's input beside Claude Code's output.
        document = summary("--home", str(RICH), "--days", "36500")
        self.assertIsInstance(document["usage"], dict)
        self.assertTrue(set(document["usage"]) <= {"claude-code", "codex", "gemini-cli", "cline"})
        self.assertNotIn("input_tokens", document["usage"])


class Window(unittest.TestCase):
    def test_the_default_window_includes_the_cutoff_but_not_the_previous_second(self):
        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp)
            path = _jsonl(home / ".claude" / "projects" / "-w" / "s1.jsonl", [
                {"type": "user", "uuid": "old", "sessionId": "s1", "cwd": "/w",
                 "timestamp": "2026-08-21T11:59:59Z", "message": {"role": "user", "content": "old"}},
                {"type": "user", "uuid": "boundary", "sessionId": "s1", "cwd": "/w",
                 "timestamp": "2026-08-21T12:00:00Z", "message": {"role": "user", "content": "boundary"}},
            ])
            # Discovery first checks file mtimes; use the same fixed clock here so
            # the event-level cutoff is what determines the result on every host.
            stamp = FixtureDatetime.now(timezone.utc).timestamp()
            transcripts.os.utime(path, (stamp, stamp))
            texts = [e["text"] for e in events("--home", str(home)) if e["kind"] == "prompt"]
            self.assertEqual(["boundary"], texts)

    def test_the_window_applies_to_events_not_just_file_mtimes(self):
        # A long-running file touched today still holds last month's turns.
        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp)
            _jsonl(home / ".claude" / "projects" / "-w" / "s1.jsonl", [
                {"type": "user", "uuid": "old", "sessionId": "s1", "cwd": "/w",
                 "timestamp": "2020-01-01T00:00:00Z", "message": {"role": "user", "content": "old"}},
                {"type": "user", "uuid": "new", "sessionId": "s1", "cwd": "/w",
                 "timestamp": NOW_TS, "message": {"role": "user", "content": "new"}},
            ])
            texts = [e["text"] for e in events("--home", str(home), "--since", "2026-01-01")
                     if e["kind"] == "prompt"]
            self.assertEqual(["new"], texts)


class RelocatedStores(unittest.TestCase):
    """`CLAUDE_CONFIG_DIR` and `CODEX_HOME` move a store; missing them reads as "no sessions"."""

    def setUp(self):
        self.saved = {k: transcripts.os.environ.get(k) for k in ("CLAUDE_CONFIG_DIR", "CODEX_HOME")}

    def tearDown(self):
        for key, value in self.saved.items():
            if value is None:
                transcripts.os.environ.pop(key, None)
            else:
                transcripts.os.environ[key] = value

    def test_the_variables_move_the_real_home_stores(self):
        with tempfile.TemporaryDirectory() as d:
            transcripts.os.environ["CLAUDE_CONFIG_DIR"] = str(Path(d) / "claude")
            transcripts.os.environ["CODEX_HOME"] = str(Path(d) / "codex")
            real = Path.home()
            self.assertEqual([Path(d) / "claude" / "projects"],
                             transcripts.ADAPTERS["claude-code"].roots(real))
            self.assertEqual([Path(d) / "codex" / "sessions"],
                             transcripts.ADAPTERS["codex"].roots(real))

    def test_the_variables_never_redirect_an_explicit_home(self):
        with tempfile.TemporaryDirectory() as d:
            transcripts.os.environ["CLAUDE_CONFIG_DIR"] = "/nonexistent/claude"
            transcripts.os.environ["CODEX_HOME"] = "/nonexistent/codex"
            home = Path(d)
            self.assertEqual([home / ".claude" / "projects"],
                             transcripts.ADAPTERS["claude-code"].roots(home))
            self.assertEqual([home / ".codex" / "sessions"],
                             transcripts.ADAPTERS["codex"].roots(home))


from unittest import mock  # noqa: E402 -- kept beside the tests that need it

# What a tool's output in an export can hold: a customer's name and a card-shaped number.
# Both synthetic, and the number assembled from fragments so nothing reads it as a card.
CUSTOMER = "Marguerite Synthetic-Customer"
CARD_SHAPED = "4000" + " 0000" * 2 + " 1234"


class ToolOutputInExports(unittest.TestCase):
    """Tool output is reduced to success and size, in the generic readers too."""

    ROWS = [
        {"role": "user", "content": "look up the customer"},
        {"role": "tool", "tool_call_id": "c1", "content": f"{CUSTOMER}, card {CARD_SHAPED}"},
        {"role": "function", "content": f"{CUSTOMER} {CARD_SHAPED}"},
        {"type": "tool_result", "is_error": True, "content": CARD_SHAPED},
        {"role": "function", "name": "lookup", "content": CUSTOMER},
    ]

    def assert_metadata_only(self, found: list[dict]) -> None:
        blob = "\n".join(json.dumps(e) for e in found)
        self.assertNotIn("Marguerite", blob)
        self.assertNotIn("1234", blob)
        results = [e for e in found if e["kind"] == "tool_result"]
        self.assertEqual(4, len(results))
        self.assertEqual([], [e for e in found if e["kind"] == "unknown"])
        for event in results:
            self.assertNotIn("text", event)
            self.assertIsInstance(event["bytes"], int)
        self.assertEqual([True, True, False, True], [e["ok"] for e in results])

    def test_a_generic_tool_output_row_is_a_result_not_an_unknown_turn(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = _jsonl(Path(tmp) / "export.jsonl", self.ROWS)
            self.assert_metadata_only(events("--input", str(path)))

    def test_a_cursor_agent_tool_output_row_is_a_result_not_an_unknown_turn(self):
        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp)
            _jsonl(home / ".cursor" / "projects" / "p" / "agent-transcripts" / "u1" / "t.jsonl",
                   self.ROWS)
            self.assert_metadata_only(
                events("--home", str(home), "--provider", "cursor-agent", "--days", "36500"))

    def test_a_tool_row_with_a_name_and_arguments_is_still_a_call(self):
        found = events("--input", str(EXPORTS / "partner-triage.jsonl"))
        self.assertEqual(["search_threads"], [e["tool"] for e in found if e["kind"] == "tool_call"])


class InputBounds(unittest.TestCase):
    """Oversized input is skipped and reported before it is parsed, never after."""

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.home = Path(self._tmp.name)

    def tearDown(self):
        self._tmp.cleanup()

    def claude(self, path: Path, text: str, uuid: str) -> dict:
        return {"type": "user", "uuid": uuid, "sessionId": path.stem, "cwd": "/w",
                "timestamp": NOW_TS, "message": {"role": "user", "content": text}}

    def test_an_oversized_line_is_skipped_and_reported(self):
        path = self.home / ".claude" / "projects" / "-w" / "s1.jsonl"
        _jsonl(path, [self.claude(path, "short and kept", "u1"),
                      self.claude(path, "OVERSIZED " * 100, "u2")])
        with mock.patch.object(transcripts, "MAX_LINE_BYTES", 400):
            out, err = run("events", "--home", str(self.home), "--days", "36500")
        self.assertIn("short and kept", out)
        self.assertNotIn("OVERSIZED", out)
        self.assertIn("1 line(s) over 400 bytes skipped", err)

    def test_an_oversized_document_is_skipped_and_reported(self):
        path = self.home / "export.json"
        path.write_text(json.dumps({"messages": [{"role": "user", "content": "BIG " * 200}]}))
        with mock.patch.object(transcripts, "MAX_FILE_BYTES", 256):
            out, err = run("events", "--input", str(path))
        self.assertNotIn("BIG", out)
        self.assertIn("over 256 bytes; not read", err)

    def test_an_oversized_prose_export_is_skipped_and_reported(self):
        path = self.home / "export.md"
        path.write_text("## User\n\n" + "BIG " * 200)
        with mock.patch.object(transcripts, "MAX_FILE_BYTES", 256):
            out, err = run("events", "--input", str(path))
        self.assertNotIn("BIG", out)
        self.assertIn("over 256 bytes; not read", err)

    def test_subagent_files_attached_to_one_session_are_capped_and_reported(self):
        session = self.home / ".claude" / "projects" / "-w"
        _jsonl(session / "s1.jsonl", [self.claude(session / "s1.jsonl", "parent", "p")])
        for i in range(5):
            agent = session / "s1" / "subagents" / f"agent-{i}.jsonl"
            _jsonl(agent, [{**self.claude(agent, f"sub {i}", f"a{i}"), "isSidechain": True,
                            "type": "assistant",
                            "message": {"role": "assistant",
                                        "content": [{"type": "text", "text": f"sub {i}"}]}}])
        with mock.patch.object(transcripts, "MAX_SUBAGENT_FILES", 2):
            document = summary("--home", str(self.home), "--days", "36500")
            found = events("--home", str(self.home), "--days", "36500")
        reasons = " | ".join(p["reason"] for p in document["problems"])
        self.assertIn("5 subagent files in one session; kept the 2 most recent, skipped 3",
                      reasons)
        self.assertEqual(2, len([e for e in found if e.get("sidechain")]))
        self.assertIn("parent", [e.get("text") for e in found])


class ClineMetadata(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.home = Path(self._tmp.name) / "home"
        self.session = self.home / ".cline" / "data" / "sessions" / "t1"
        self.session.mkdir(parents=True)
        (self.session / "t1.messages.json").write_text(json.dumps(
            {"messages": [{"role": "user", "content": "hello"}]}))

    def tearDown(self):
        self._tmp.cleanup()

    def test_a_symlinked_metadata_file_is_not_followed(self):
        outside = Path(self._tmp.name) / "outside.json"
        outside.write_text(json.dumps({"cwd": "/somewhere/the/user/never/named"}))
        (self.session / "t1.json").symlink_to(outside)
        document = discover("--home", str(self.home), "--days", "36500")
        self.assertNotIn("/somewhere/the/user/never/named", json.dumps(document["found"]))
        self.assertIn("resolves outside the store",
                      " ".join(p["reason"] for p in document["problems"]))
        self.assertEqual(1, document["found"][0]["sessions"])

    def test_a_non_string_cwd_does_not_crash_discovery(self):
        (self.session / "t1.json").write_text(json.dumps({"cwd": {"not": "a string"}}))
        document = discover("--home", str(self.home), "--days", "36500")
        self.assertEqual([], document["found"][0]["projects"])


# --------------------------------------------------------------------------
# Second external review. Each case was reproduced against the script before it was
# fixed; the test is the reproduction.
# --------------------------------------------------------------------------

import ast  # noqa: E402 -- kept beside the tests that need it
import os  # noqa: E402

# Assembled from fragments, like `_STRIPE_SHAPED`, so no scanner reads it as a value.
SECRET_WORD = "not" + "-a-real-" + "word-0004"


def _claude(uuid: str, kind: str, content, ts: str = NOW_TS, **extra) -> dict:
    return {"type": kind, "uuid": uuid, "sessionId": "s", "cwd": "/w", "timestamp": ts,
            "message": {"role": kind, "content": content, **extra.pop("message", {})}, **extra}


class LongQuotedCredentials(unittest.TestCase):
    def test_a_quoted_credential_longer_than_256_characters_is_redacted_whole(self):
        # The quoted form was capped at 256 characters. Past that it did not match at
        # all, and a value with spaces in it matched no other rule either.
        value = " ".join([SECRET_WORD] * 40)
        text, _ = transcripts.clip('config api_key: "' + value + '" done', 600)
        self.assertNotIn(SECRET_WORD, text)
        self.assertIn("<redacted>", text)
        self.assertTrue(text.endswith("done"), text)

    def test_a_quoted_credential_cut_off_by_the_scan_window_is_redacted_to_the_end(self):
        value = " ".join([SECRET_WORD] * 400)
        text, _ = transcripts.clip('the password: "' + value + '"', 50)
        self.assertNotIn(SECRET_WORD, text)


class UnrecognisedRolesInExports(unittest.TestCase):
    def found(self, rows: list) -> list[dict]:
        with tempfile.TemporaryDirectory() as tmp:
            return events("--input", str(_jsonl(Path(tmp) / "export.jsonl", rows)))

    def test_a_role_that_looks_like_tool_output_is_a_result_with_no_body(self):
        found = self.found([
            {"role": "user", "content": "look up the customer"},
            {"role": "function_output", "content": f"{CUSTOMER}, card {CARD_SHAPED}"},
            {"role": "tool_response", "content": CUSTOMER},
            {"role": "lookup_result", "name": "lookup", "arguments": {"q": 1},
             "content": CUSTOMER},
        ])
        blob = "\n".join(json.dumps(e) for e in found)
        self.assertNotIn("Marguerite", blob)
        self.assertEqual(3, sum(1 for e in found if e["kind"] == "tool_result"))
        self.assertEqual(["look up the customer"],
                         [e["text"] for e in found if e["kind"] == "prompt"])

    def test_a_record_of_an_unknown_role_is_counted_but_not_quoted(self):
        found = self.found([
            {"role": "observation", "content": CUSTOMER},
            {"speaker": "nobody", "body": CUSTOMER},
            {"role": "assistant", "content": "here is what I found"},
        ])
        unknown = [e for e in found if e["kind"] == "unknown"]
        self.assertEqual(2, len(unknown))
        self.assertNotIn("Marguerite", "\n".join(json.dumps(e) for e in found))
        for event in unknown:
            self.assertNotIn("text", event)
        self.assertEqual(["here is what I found"],
                         [e["text"] for e in found if e["kind"] == "assistant"])

    def test_prose_before_the_first_speaker_heading_is_counted_but_not_quoted(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "export.md"
            path.write_text(f"{CUSTOMER}\n\n## User\n\nwhat changed\n")
            found = events("--input", str(path))
        self.assertNotIn("Marguerite", "\n".join(json.dumps(e) for e in found))
        self.assertEqual(["what changed"], [e["text"] for e in found if e["kind"] == "prompt"])
        self.assertEqual(1, sum(1 for e in found if e["kind"] == "unknown"))


class MetadataAtTheOutputBoundary(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.home = Path(self._tmp.name)
        _jsonl(self.home / ".claude" / "projects" / "-w" / "s1.jsonl", [
            {**_claude("u1", "user", "go"), "cwd": "/w/DB_PASSWORD=" + SECRET_WORD},
            _claude("a1", "assistant", [
                {"type": "tool_use", "id": "t1", "name": "api_key=" + SECRET_WORD,
                 "input": {"token=" + SECRET_WORD: "x"}}],
                message={"model": "m" * 5000}),
        ])

    def tearDown(self):
        self._tmp.cleanup()

    def test_tool_model_and_project_names_are_redacted_and_bounded_in_emitted_events(self):
        found = events("--home", str(self.home), "--days", "36500")
        blob = "\n".join(json.dumps(e) for e in found)
        self.assertNotIn(SECRET_WORD, blob)
        for event in found:
            for key, value in event.items():
                if isinstance(value, str) and key != "text":
                    self.assertLessEqual(len(value), transcripts.META_CHARS + 1, key)
        self.assertIn("<redacted>", [e.get("tool") for e in found])

    def test_the_summary_carries_the_same_redacted_names(self):
        blob = json.dumps(summary("--home", str(self.home), "--days", "36500"))
        self.assertNotIn(SECRET_WORD, blob)
        self.assertNotIn("m" * 300, blob)


class InvalidDates(unittest.TestCase):
    def test_an_unparsable_since_or_until_is_an_error_not_the_default_window(self):
        for flag in ("--since", "--until"):
            err = io.StringIO()
            with redirect_stderr(err), self.assertRaises(SystemExit) as caught:
                transcripts.main(["summary", "--home", str(BARREN), flag, "last tuesday"])
            self.assertNotEqual(0, caught.exception.code, flag)
            self.assertIn(flag, err.getvalue())
            self.assertIn("last tuesday", err.getvalue())


class FiltersOnExports(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self._tmp.name)
        self.timed = _jsonl(self.dir / "timed.jsonl", [
            {"role": "user", "content": "old turn", "timestamp": "2020-01-01T00:00:00Z"},
            {"role": "user", "content": "new turn", "timestamp": NOW_TS},
        ])

    def tearDown(self):
        self._tmp.cleanup()

    def test_an_explicit_window_applies_to_an_export_with_timestamps(self):
        found = events("--input", str(self.timed), "--since", "2026-01-01")
        self.assertEqual(["new turn"], [e["text"] for e in found if e["kind"] == "prompt"])

    def test_the_default_window_does_not_hide_a_file_the_user_named(self):
        found = events("--input", str(self.timed))
        self.assertEqual(["old turn", "new turn"],
                         [e["text"] for e in found if e["kind"] == "prompt"])

    def test_max_sessions_caps_exports_and_says_it_capped(self):
        other = _jsonl(self.dir / "other.jsonl", [{"role": "user", "content": "other"}])
        document = summary("--input", str(self.timed), "--input", str(other),
                           "--max-sessions", "1")
        self.assertEqual(1, document["totals"]["sessions"])
        self.assertIn("kept the 1", " ".join(p["reason"] for p in document["problems"]))

    def test_a_filter_that_cannot_apply_to_an_export_is_reported(self):
        prose = self.dir / "export.md"
        prose.write_text("## User\n\nwhat changed\n")
        _, err = run("events", "--input", str(prose), "--since", "2026-01-01",
                     "--project", "helio")
        self.assertIn("--project does not apply", err)
        self.assertIn("no timestamp", err)


class UserTagsSurviveWrapperStripping(unittest.TestCase):
    def test_only_the_wrappers_a_provider_injects_are_removed(self):
        self.assertEqual("<question>what is x</question>",
                         transcripts.typed_text("<question>what is x</question>"))
        self.assertIn("client records",
                      transcripts.typed_text("Analyze <document>client records</document>"))
        self.assertEqual("do it", transcripts.typed_text(
            "<system-reminder>injected</system-reminder> do it"
            "<environment_context><cwd>/w</cwd></environment_context>"))

    def test_a_prompt_made_of_a_user_tag_is_still_a_prompt(self):
        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp)
            _jsonl(home / ".claude" / "projects" / "-w" / "s1.jsonl",
                   [_claude("u1", "user", "<question>why is the build red</question>")])
            texts = [e["text"] for e in events("--home", str(home), "--days", "36500")
                     if e["kind"] == "prompt"]
        self.assertEqual(["<question>why is the build red</question>"], texts)


class ReplayedToolsAcrossFiles(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.home = Path(self._tmp.name)

    def tearDown(self):
        self._tmp.cleanup()

    def test_a_resumed_claude_code_session_does_not_recount_its_tools_or_usage(self):
        history = [
            _claude("u1", "user", "run the tests"),
            {**_claude("a1", "assistant", [{"type": "tool_use", "id": "toolu_1",
                                             "name": "Bash", "input": {"command": "make"}}],
                       message={"usage": {"output_tokens": 10}}), "requestId": "r1"},
            _claude("u2", "user", [{"type": "tool_result", "tool_use_id": "toolu_1",
                                    "content": "ok"}]),
        ]
        project = self.home / ".claude" / "projects" / "-w"
        _jsonl(project / "s1.jsonl", history)
        _jsonl(project / "s2.jsonl", [*history,
            # An identical call with its own id is a second call, not a replay.
            {**_claude("a2", "assistant", [{"type": "tool_use", "id": "toolu_2",
                                             "name": "Bash", "input": {"command": "make"}}],
                       message={"usage": {"output_tokens": 5}}), "requestId": "r2"},
        ])
        document = summary("--home", str(self.home), "--days", "36500")
        self.assertEqual(2, document["events_by_kind"]["tool_call"])
        self.assertEqual(1, document["events_by_kind"]["tool_result"])
        self.assertEqual({"output_tokens": 15, "requests": 2}, document["usage"]["claude-code"])

    def test_a_forked_codex_rollout_does_not_recount_the_calls_it_replays(self):
        call = _rec("response_item", {"type": "function_call", "name": "shell",
                                      "call_id": "call_1", "arguments": "{}"})
        output = _rec("response_item", {"type": "function_call_output", "call_id": "call_1",
                                        "output": "done"})
        _codex(self.home, f"rollout-2026-09-20T10-00-00-{UUID_B}.jsonl",
               [_codex_meta(UUID_B), call, output])
        _codex(self.home, f"rollout-2026-09-20T11-00-00-{UUID_A}.jsonl", [
            _codex_meta(UUID_A, forked_from_id=UUID_B), _codex_meta(UUID_B), call, output,
            _rec("response_item", {"type": "function_call", "name": "shell",
                                   "call_id": "call_2", "arguments": "{}"}),
        ])
        document = summary("--home", str(self.home), "--provider", "codex", "--days", "36500")
        self.assertEqual(2, document["events_by_kind"]["tool_call"])
        self.assertEqual(1, document["events_by_kind"]["tool_result"])


class CodexSubagentsAndTheSessionCap(unittest.TestCase):
    def test_subagent_rollouts_do_not_displace_their_parent_under_the_cap(self):
        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp)
            parent = _codex(home, f"rollout-2026-09-20T10-00-00-{UUID_A}.jsonl", [
                _codex_meta(UUID_A),
                _rec("event_msg", {"type": "user_message", "message": "the parent's prompt"}),
            ])
            os.utime(parent, (1_000_000, 1_000_000))
            for i in range(3):
                child = f"019a0000-0000-7000-8000-00000000010{i}"
                _codex(home, f"rollout-2026-09-20T10-0{i + 1}-00-{child}.jsonl", [
                    _codex_meta(child, source={"subagent": {"thread_spawn": {
                        "parent_thread_id": UUID_A}}}),
                    _rec("event_msg", {"type": "agent_message", "message": f"child {i}"}),
                ])
            argv = ("--home", str(home), "--provider", "codex", "--days", "36500",
                    "--max-sessions", "1")
            document = discover(*argv)
            found = events(*argv)
        self.assertEqual(1, document["found"][0]["sessions"])
        self.assertIn("the parent's prompt", [e.get("text") for e in found])
        self.assertEqual({UUID_A}, {e["session"] for e in found})


class EventBudget(unittest.TestCase):
    def test_a_run_stops_at_the_event_budget_and_says_so(self):
        with mock.patch.object(transcripts, "MAX_EVENTS", 3):
            document = summary("--home", str(RICH), "--days", "36500")
        self.assertLessEqual(document["totals"]["events"], 3)
        self.assertIn("event budget of 3 reached",
                      " ".join(p["reason"] for p in document["problems"]))

    def test_tool_pairs_are_the_same_without_per_session_lists(self):
        # The pairs are now counted as events arrive, from the last tool per session.
        document = summary("--home", str(RICH))
        pairs = {tuple(entry["pair"]): entry["count"] for entry in document["tool_pairs"]}
        self.assertGreater(pairs[("Edit", "Bash")], 1)


# Imports the script may make. Everything here reads local files or computes; nothing
# here opens a connection or starts a process.
ALLOWED_IMPORTS = frozenset({
    "__future__", "argparse", "collections", "dataclasses", "datetime", "hashlib", "json",
    "os", "pathlib", "re", "sys", "typing",
})
# Named so that widening `ALLOWED_IMPORTS` to one of these is a visible, reviewable act.
NETWORK_OR_PROCESS = frozenset({
    "asyncio", "ctypes", "ftplib", "http", "imaplib", "importlib", "multiprocessing",
    "poplib", "requests", "select", "selectors", "smtplib", "socket", "socketserver", "ssl",
    "subprocess", "telnetlib", "urllib", "xmlrpc",
})
# `os` is allowed for `environ`; these are the parts of it that start a process.
OS_PROCESS_CALLS = frozenset({
    "system", "popen", "fork", "forkpty", "posix_spawn", "posix_spawnp",
    *(f"exec{s}" for s in ("l", "le", "lp", "lpe", "v", "ve", "vp", "vpe")),
    *(f"spawn{s}" for s in ("l", "le", "lp", "lpe", "v", "ve", "vp", "vpe")),
})


def import_violations(source: str) -> list[str]:
    """Every import, dynamic import, or process call in `source` outside the allowlist."""
    found = []
    for node in ast.walk(ast.parse(source)):
        if isinstance(node, ast.Import):
            roots = [alias.name.split(".")[0] for alias in node.names]
        elif isinstance(node, ast.ImportFrom):
            roots = [(node.module or "").split(".")[0]] if not node.level else []
        else:
            roots = []
        found += [f"import {root}" for root in roots if root not in ALLOWED_IMPORTS]
        if isinstance(node, ast.Name) and node.id in ("__import__", "importlib"):
            found.append(node.id)
        if isinstance(node, ast.Attribute) and node.attr == "__import__":
            found.append("__import__")
        if (isinstance(node, ast.Attribute) and node.attr in OS_PROCESS_CALLS
                and isinstance(node.value, ast.Name) and node.value.id == "os"):
            found.append(f"os.{node.attr}")
    return found


class NoNetworkNoProcess(unittest.TestCase):
    def test_the_allowlist_excludes_every_network_and_process_module(self):
        self.assertEqual(set(), ALLOWED_IMPORTS & NETWORK_OR_PROCESS)

    def test_the_script_imports_only_the_allowlist_and_nothing_dynamically(self):
        # The privacy claim in SKILL.md is that this runs locally and sends nothing. A
        # substring check for `import socket` passed `from urllib import request`.
        self.assertEqual([], import_violations(SCRIPT.read_text(encoding="utf-8")))

    def test_the_check_rejects_the_forms_the_substring_check_missed(self):
        for source in ("from urllib import request", "import http.client as h",
                       "__import__('socket')", "import importlib",
                       "import os\nos.system('x')", "from ssl import wrap_socket"):
            self.assertTrue(import_violations(source), source)


if __name__ == "__main__":
    unittest.main(verbosity=2, buffer=False)

#!/usr/bin/env python3
"""Discover, normalise and summarise local coding-agent transcripts.

This runs entirely on the machine it is invoked on. It opens no socket, sends
nothing anywhere, and writes nothing outside the path it is told to write to.
The audit that consumes its output is performed by the calling agent against
these local files; no transcript content is ever sent to TypeSafe or to `jev`.

Why a script rather than instructions. Provider formats are undocumented
internal state that changes between releases, and re-deriving them from a
sample on every invocation is how an agent invents a key path that does not
exist. The mapping lives here, in one place, next to the notes that say where
each field was observed, so that changing it is an edit rather than a guess.

Three subcommands:

    discover   which providers are present, how many sessions, over what dates
    events     normalised events as JSONL, one per line
    summary    deterministic aggregates: tool histograms, repeated sequences

Privacy defaults are deliberately strict, because a transcript store holds
prompts, source code, customer data, internal documents, and whatever a tool
happened to print. Text is clipped, obvious credential shapes are replaced,
and tool results are reduced to metadata unless asked for. Widening any of
that is an explicit flag, so it appears in the command the user can read.

Adapters are best-effort by construction. A provider that changed its format
yields fewer events or none; it never raises, and what failed is reported in
`problems` rather than swallowed. An adapter that silently returns nothing is
worse than one that says it could not read anything.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sys
from collections import Counter, defaultdict
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any, Callable, Iterable, Iterator

EVENT_SCHEMA = "jev.retro.event/v1"
SUMMARY_SCHEMA = "jev.retro.summary/v1"
DISCOVERY_SCHEMA = "jev.retro.discovery/v1"

DEFAULT_MAX_CHARS = 600
DEFAULT_ARG_CHARS = 200
DEFAULT_DAYS = 30

# Input bounds. `clip` bounds what is *emitted*, but only after a line or a document has
# been read and parsed whole: one 80 MB line peaked at 217 MB of memory to produce 600
# characters. Anything over these is skipped and reported, never parsed. They are read
# at call time, not bound as defaults, so a test can lower them.
MAX_LINE_BYTES = 16 * 1024 * 1024
#: Applies to files read whole -- JSON documents and prose exports. JSONL is streamed a
#: line at a time, so its memory is bounded by `MAX_LINE_BYTES`, and a long real session
#: legitimately exceeds any per-file figure small enough to matter here.
MAX_FILE_BYTES = 16 * 1024 * 1024
#: Subagent transcripts attached to one parent session. They carry the parent's id, so
#: `--max-sessions` never counted them, and one session could pull in any number.
MAX_SUBAGENT_FILES = 200
#: Events read in one run, across every file. The per-line and per-file caps bound one
#: record; nothing bounded the run, and the replay-dedupe keys and summary aggregates grow
#: with every event, so a multi-GB store could still exhaust memory a line at a time. This
#: is far above any real window -- a heavy month is tens of thousands -- and reaching it is
#: reported as a problem, never a silent stop.
MAX_EVENTS = 2_000_000
#: Longest untrusted metadata string emitted -- a tool, model, project or session name.
META_CHARS = 200

# Kinds a normalised event can carry. Deliberately short: the goal is workflow
# pattern discovery, not archival, and every kind here is one an audit reasons
# about. Thinking blocks, diffs, attachments and UI bookkeeping are dropped.
KINDS = (
    "session", "prompt", "assistant", "tool_call", "tool_result", "subagent", "skill",
    # Emitted only by the generic adapter, for a record whose role it cannot
    # establish. It is a kind rather than a guess, and it is in this tuple
    # rather than outside it because the default filter in `events` is built
    # from this tuple: a kind missing here is a turn that disappears.
    "unknown",
)


# --------------------------------------------------------------------------
# Redaction and clipping.
#
# These run on every string that leaves this file. They are a backstop, not a
# guarantee: a secret in an unusual shape will pass. The real control is that
# nothing here is transmitted anywhere.
# --------------------------------------------------------------------------

_SECRET_PATTERNS = [
    # Vendor-prefixed keys. The prefixes are public; the bodies are not. The separator
    # is a class, not a literal hyphen: Stripe uses `sk_live_…` and an earlier version
    # of this pattern, which required `sk-`, let every real Stripe key through.
    re.compile(r"\b(?:sk|pk|rk|ak)[-_][A-Za-z0-9_-]{16,}"),
    re.compile(r"\bghp_[A-Za-z0-9]{20,}"),
    re.compile(r"\bgh[oprsu]_[A-Za-z0-9]{20,}"),
    # GitHub fine-grained, GitLab, Slack app-level, npm and Hugging Face tokens. Each
    # body is required to be long and unpunctuated, so `npm_config_cache` and similar
    # identifiers that merely share a prefix are left alone.
    re.compile(r"\bgithub_pat_[A-Za-z0-9_]{20,}"),
    re.compile(r"\bglpat-[A-Za-z0-9_-]{20,}"),
    re.compile(r"\bxox[abprs]-[A-Za-z0-9-]{10,}"),
    re.compile(r"\bxapp-[A-Za-z0-9-]{10,}"),
    re.compile(r"\bnpm_[A-Za-z0-9]{30,}"),
    re.compile(r"\bhf_[A-Za-z0-9]{30,}"),
    # `ASIA…` is the temporary (STS) form of an AWS access key id; matching only
    # `AKIA…` let every session credential through.
    re.compile(r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b"),
    re.compile(r"\bAIza[0-9A-Za-z_-]{30,}"),
    re.compile(r"\bey[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}"),
    # PEM and PGP private keys: replace the whole body, not just the header. A block
    # with no END line is redacted to the end of the field, because that is exactly
    # what a key cut off by `clip`'s scan window, or pasted half-way, looks like --
    # requiring the END line let the whole body through.
    re.compile(
        r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY(?: BLOCK)?-----.*?"
        r"(?:-----END [A-Z0-9 ]*PRIVATE KEY(?: BLOCK)?-----|\Z)",
        re.S,
    ),
    # `api_key: "…"`, `Authorization: Bearer …`, and the shape that actually dominates a
    # transcript: `AWS_SECRET_ACCESS_KEY=…`, `DB_PASSWORD=…`, `STRIPE_SECRET=…`.
    #
    # The keyword was once wrapped in `\b` on both sides, which looks right and is
    # wrong: `_` is a word character, so `\b` never fires *inside* an identifier and
    # every SCREAMING_SNAKE_CASE variable name passed through untouched. The keyword is
    # therefore allowed to sit anywhere within the surrounding identifier.
    #
    # An optional quote may close the key before the separator: `{"api_key": "…"}` and
    # Python's `{'password': '…'}` put one between keyword and colon, and without it
    # the most common serialised form of a credential was the one form not caught. A
    # quoted value is taken to its closing quote, spaces and all; an unquoted one must
    # still be six characters, so `token: yes` in prose is not mistaken for a secret.
    #
    # The quoted value has no length cap. It was capped at 256 characters, and a longer
    # value -- a service-account blob, a passphrase with spaces -- then matched no rule
    # at all and `clip` emitted its head. Nothing is lost by lifting it: the two
    # alternatives in the quoted body are disjoint, so the match is linear, and `clip`
    # has already bounded the field. A quote with no closing quote is redacted to the end
    # of the field, for the same reason as a PEM block with no END line: that is what a
    # value cut off by `clip`'s scan window looks like.
    #
    # The lookbehind starts a match only where an identifier starts. Without it every
    # position inside a long run of identifier characters retried the greedy prefix, which
    # is quadratic: one 5,000-character model name took 0.7 s to redact. It changes no
    # match, because the prefix can absorb whatever a later start would have skipped.
    re.compile(
        r"(?i)(?<![A-Za-z0-9_.\[\]-])[A-Za-z0-9_.\[\]-]*"
        r"(?:authorization|api[-_]?key|secret|token|password|passwd|credential|passphrase)"
        r"[A-Za-z0-9_.\[\]-]*[\"']?\s*[:=]\s*"
        r"(?:\"(?:[^\"\\]|\\.)*(?:\"|\Z)|'(?:[^'\\]|\\.)*(?:'|\Z)"
        r"|[\"']?(?:bearer|token|basic)?\s*[^\s\"',;]{6,})"
    ),
    # `--password VALUE`, `--api-key VALUE`. The flag is kept, so an audit can still see
    # that one was passed. The flag must *end* in the keyword: `--password-file path`
    # and `--token-stdin` name where a secret is, not the secret, and eating the path
    # would hide a workflow fact for no gain.
    re.compile(
        r"(?i)(?P<keep>(?<![\w-])--[A-Za-z0-9-]*"
        r"(?:password|passwd|passphrase|token|secret|api[-_]?key)(?![A-Za-z0-9_-])\s+)"
        r"(?:\"(?:[^\"\\]|\\.)*\"|'[^']*'|[^\s\"'-][^\s\"']{5,})"
    ),
    # `curl -u user:pass`, `--user=user:pass`. A bare `uid:gid` such as Docker's
    # `-u 1000:1000` is not a credential and is left alone.
    re.compile(
        r"(?P<keep>(?<![\w-])(?:-u|--user)(?:\s+|=)?)"
        r"(?!\d+:\d+(?:\s|$))[\"']?[^\s\"':-][^\s\"':]*:[^\s\"']+[\"']?"
    ),
    # A credential in a URL's authority: `postgres://user:pw@host/db`. No keyword sits
    # anywhere near it, so nothing above can reach it.
    re.compile(r"\b[a-zA-Z][a-zA-Z0-9+.-]*://[^\s/:@]+:[^\s/@]+@"),
]

_REDACTED = "<redacted>"


def _redaction(match: re.Match) -> str:
    """The replacement for one match: the `keep` group, if the pattern has one, then the marker."""
    return (match.groupdict().get("keep") or "") + _REDACTED

# How much of an over-long field is scanned before truncation. Large enough that
# collapsing whitespace cannot starve a normal field of content, small enough that one
# adversarial line cannot exhaust memory.
_SCAN_MULTIPLE = 8
_SCAN_FLOOR = 4096


def redact(text: str) -> str:
    """Replace credential-shaped substrings. Applied to every emitted string."""
    for pattern in _SECRET_PATTERNS:
        text = pattern.sub(_redaction, text)
    return text


def clip(value: Any, limit: int) -> tuple[str, bool]:
    """Render `value` as a redacted string of at most `limit` characters.

    Returns the text and whether it was shortened, so a consumer can tell the
    difference between a short prompt and a long one it is only seeing the head
    of. Collapsing whitespace is not cosmetic: it keeps one clipped prompt on
    one JSONL line and stops a pasted file dominating the budget.
    """
    if value is None:
        return "", False
    if not isinstance(value, str):
        try:
            value = json.dumps(value, ensure_ascii=False, sort_keys=True)
        except (TypeError, ValueError):
            value = str(value)
    # Bound the work, not only the output. `split()` over a 100 MB line materialises
    # millions of small strings and every pattern below then runs over all of it, so a
    # single corrupted or hostile line could drive a modest process towards an OOM while
    # producing at most `limit` characters. Slice first, generously: a credential that
    # begins past this point cannot appear in the output either.
    head = value[: max(limit, 0) * _SCAN_MULTIPLE + _SCAN_FLOOR]
    value = redact(" ".join(head.split()))
    if len(value) <= limit:
        return value, False
    return value[:limit], True


def safe_args(raw: Any, limit: int) -> dict[str, Any]:
    """Reduce a tool's arguments to a bounded, readable digest.

    Which tool was called with roughly what shape is the signal an audit needs;
    the full argument is frequently a file, a diff, or a subagent prompt, and
    reproducing it here would put the contents of the user's work into the
    caller's context for no analytical gain. Scalars are clipped, containers
    become their type and size, and the key count is capped.
    """
    if not isinstance(raw, dict):
        text, truncated = clip(raw, limit)
        return {"_": text} if not truncated else {"_": text, "_truncated": True}
    out: dict[str, Any] = {}
    for key in sorted(raw)[:8]:
        value = raw[key]
        if isinstance(value, (dict, list)):
            out[key] = f"<{type(value).__name__} len={len(value)}>"
        elif isinstance(value, (int, float, bool)) or value is None:
            out[key] = value
        else:
            # Redact the pair, not the value. The keyword patterns match a keyword next
            # to its value, and splitting a tool call's arguments into separate
            # key/value entries breaks exactly that adjacency -- so a credential passed
            # as `password="…"` was emitted in full, while the same bytes inside one
            # string would have been replaced.
            pair, truncated = clip(f"{key}={value}", limit + len(key) + 1)
            prefix = f"{key}="
            text = pair[len(prefix):] if pair.startswith(prefix) else _REDACTED
            out[key] = text + ("…" if truncated else "")
    if len(raw) > 8:
        out["_more_keys"] = len(raw) - 8
    return out


# --------------------------------------------------------------------------
# Time.
# --------------------------------------------------------------------------


def parse_ts(value: Any) -> datetime | None:
    """Best-effort timestamp parse across the shapes providers actually use."""
    if value is None:
        return None
    if isinstance(value, (int, float)):
        # Milliseconds since the epoch are common; seconds are too. Anything
        # past year 5000 read as seconds is milliseconds.
        seconds = value / 1000.0 if value > 1e11 else float(value)
        try:
            return datetime.fromtimestamp(seconds, tz=timezone.utc)
        except (OverflowError, OSError, ValueError):
            return None
    if not isinstance(value, str):
        return None
    text = value.strip().replace("Z", "+00:00")
    try:
        parsed = datetime.fromisoformat(text)
    except ValueError:
        return None
    return parsed if parsed.tzinfo else parsed.replace(tzinfo=timezone.utc)


def iso(moment: datetime | None) -> str | None:
    return moment.astimezone(timezone.utc).isoformat().replace("+00:00", "Z") if moment else None


# --------------------------------------------------------------------------
# Session references and adapters.
# --------------------------------------------------------------------------


@dataclass
class SessionRef:
    provider: str
    path: Path
    session_id: str
    project: str | None = None
    mtime: float = 0.0
    extra: dict[str, Any] | None = None


class Problem(dict):
    """A source that could not be read. Reported, never raised."""

    def __init__(self, provider: str, path: Any, reason: str) -> None:
        super().__init__(provider=provider, path=str(path), reason=reason)


def read_jsonl(path: Path, problems: list[Problem], provider: str) -> Iterator[dict]:
    """Yield the parseable objects of a JSONL file, skipping the rest.

    A truncated final line is the normal state of a session that is still open,
    so a bad line is counted and stepped over rather than failing the file.
    """
    bad = 0
    good = 0
    oversized = 0
    limit = MAX_LINE_BYTES
    try:
        # Binary, a bounded `readline` at a time. Iterating a text handle reads each line
        # whole before anything can look at its length, which is the unbounded read the
        # cap exists to prevent.
        with path.open("rb") as handle:
            first = True
            while True:
                chunk = handle.readline(limit + 1)
                if not chunk:
                    break
                if len(chunk) > limit and not chunk.endswith(b"\n"):
                    # Drain the rest of the line without keeping it.
                    while chunk and not chunk.endswith(b"\n"):
                        chunk = handle.readline(1024 * 1024)
                    oversized += 1
                    first = False
                    continue
                # A byte-order mark glued to the first line made it unparsable, and for
                # Codex the first line is the session header -- so the project and the
                # session id were silently lost along with it.
                if first:
                    chunk = chunk.removeprefix(b"\xef\xbb\xbf")
                    first = False
                line = chunk.decode("utf-8", errors="replace").strip()
                if not line:
                    continue
                try:
                    record = json.loads(line)
                except (ValueError, RecursionError):
                    bad += 1
                    continue
                # A line that is valid JSON but not an object is still not a record.
                # Stepping over it without counting it made a file of `[1,2,3]` lines
                # look like an empty session rather than an unreadable one.
                if not isinstance(record, dict):
                    bad += 1
                    continue
                good += 1
                yield record
    except OSError as error:
        problems.append(Problem(provider, path, f"unreadable: {error.strerror or error}"))
        return
    if oversized:
        problems.append(
            Problem(provider, path, f"{oversized} line(s) over {limit} bytes skipped, not parsed")
        )
    if bad:
        problems.append(Problem(provider, path, f"{bad} unparsable line(s) skipped"))
    elif not good and not oversized:
        problems.append(Problem(provider, path, "no records"))


def read_bounded(path: Path, problems: list[Problem], provider: str) -> str | None:
    """The text of a file that has to be read whole, or None if it is over the cap.

    Reads at most one byte past the cap rather than trusting `stat`, so a file that grows
    while it is read, or lies about its size, still cannot push past it.
    """
    limit = MAX_FILE_BYTES
    try:
        with path.open("rb") as handle:
            data = handle.read(limit + 1)
    except OSError as error:
        problems.append(Problem(provider, path, f"unreadable: {error.strerror or error}"))
        return None
    if len(data) > limit:
        problems.append(Problem(provider, path, f"over {limit} bytes; not read"))
        return None
    # `utf-8-sig` strips a byte-order mark, which would otherwise make the document
    # unparsable.
    return data.decode("utf-8-sig", errors="replace")


def read_json(path: Path, problems: list[Problem], provider: str) -> Any:
    text = read_bounded(path, problems, provider)
    if text is None:
        return None
    try:
        return json.loads(text)
    except (ValueError, RecursionError) as error:
        problems.append(Problem(provider, path, f"unreadable: {type(error).__name__}"))
        return None


def as_str(value: Any) -> str | None:
    """A field that should be a string, or None.

    Every name this script aggregates on -- a project, a tool, an agent type -- ends up
    as a set member or a counter key. A provider that writes a number or an object there
    turned a single odd record into an `unhashable type` crash that lost the whole run.
    """
    if value is None:
        return None
    if isinstance(value, str):
        return value
    if isinstance(value, (int, float, bool)):
        return str(value)
    return None


_UUID_TAIL = re.compile(
    r"([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})$", re.I
)


def uuid_tail(stem: str) -> str:
    """The session UUID at the end of a filename stem, or the stem itself.

    `rsplit("-", 5)` kept only the last twelve hex digits of the UUID, which matters
    exactly when the in-file header is missing and the filename is all there is.
    """
    match = _UUID_TAIL.search(stem)
    return match.group(1) if match else stem


# A turn made entirely of wrapper elements -- `<environment_context>…</…>`,
# `<subagent_notification>…`, `<task-notification>…` -- is something the harness
# injected, not something a person typed. Counting it as a prompt inflated every
# frequency the report quotes; on real data it was 37% of Codex "prompts" and 40% of
# Claude Code ones.
#
# Only the tags a provider injects are removed, by name. Removing *every* XML-like
# element also removed what people type: `<question>…</question>` became an empty turn
# and `Analyze <document>client records</document>` lost the records. A tag not listed
# here is the person's text and is kept verbatim. The Codex names are the ones in
# references/providers.md; the Claude Code ones are its slash-command, local-command,
# shell-escape, hook and reminder wrappers.
_INJECTED_TAGS = (
    # Codex.
    "environment_context", "user_instructions", "subagent_notification",
    "recommended_plugins", "turn_aborted",
    # Claude Code.
    "system-reminder", "task-notification", "user-prompt-submit-hook",
    "command-name", "command-message", "command-args",
    "local-command-stdout", "local-command-stderr", "local-command-caveat",
    "bash-input", "bash-stdout", "bash-stderr",
)
_TAG_NAMES = "|".join(re.escape(tag) for tag in _INJECTED_TAGS)
# `(?![\w-])` rather than `\b`: `\b` fires between `name` and `-x`, so `<command-name-x>`
# would have been taken for `<command-name>`.
_WRAPPER = re.compile(
    rf"<({_TAG_NAMES})(?![\w-])[^>]*>.*?</\1\s*>|<(?:{_TAG_NAMES})(?![\w-])[^>]*/>",
    re.S | re.I,
)


# Wrappers around text the person supplied themselves. These are unwrapped, not removed:
# pasting a document into a prompt is still the person's prompt. The closing tag may
# repeat the opening tag's attributes, so it is matched loosely.
_USER_CONTENT = re.compile(
    r"<(pasted_content|pasted-content|attachment)\b[^>]*>(.*?)</\1\b[^>]*>", re.S | re.I
)


def typed_text(text: str) -> str:
    """What is left of a user turn once injected wrapper elements are removed."""
    text = _USER_CONTENT.sub(lambda m: f" {m.group(2)} ", text)
    return _WRAPPER.sub(" ", text).strip()


def within(root: Path, path: Path) -> bool:
    """Is `path`, fully resolved, still inside `root`?

    `Path.glob` follows symlinks, so a link planted inside a transcript store points
    this script at a file the user never named -- another project's source, a key, a
    file belonging to someone else. `AGENTS.md` §4: do not follow a symlink into a
    location the user did not name. A path that cannot be resolved is treated as
    outside, because "I could not tell" is not a reason to read it.
    """
    try:
        return path.resolve().is_relative_to(root.resolve())
    except (OSError, RuntimeError, ValueError):
        return False


class Adapter:
    """One provider's storage layout and record mapping.

    `discover` locates sessions cheaply, from paths and mtimes alone, so that a
    store with tens of thousands of files can be date-filtered without opening
    them. `events` is what opens a file.
    """

    name = "abstract"
    #: Where this provider keeps sessions, relative to the home directory.
    root_hint = ""

    def roots(self, home: Path) -> list[Path]:
        raise NotImplementedError

    def discover(self, home: Path, problems: list[Problem]) -> list[SessionRef]:
        raise NotImplementedError

    def events(self, ref: SessionRef, problems: list[Problem]) -> Iterator[dict]:
        raise NotImplementedError


def base_event(ref: SessionRef, seq: int, kind: str, ts: datetime | None, **fields: Any) -> dict:
    event = {
        "schema": EVENT_SCHEMA,
        "provider": ref.provider,
        "session": ref.session_id,
        "project": ref.project,
        "seq": seq,
        "ts": iso(ts),
        "kind": kind,
    }
    event.update({k: v for k, v in fields.items() if v is not None})
    return event


# The fields this file writes itself. Every other field came out of a transcript.
_TRUSTED_FIELDS = frozenset({"schema", "provider", "kind", "ts", "seq"})
# The only kinds whose text is a person's or an agent's prose. Everything else -- a tool
# result, a turn whose role could not be established -- is counted, never quoted: an
# unrecognised role is as likely to be tool output under a name this file has not seen
# as it is to be a turn, and tool output is where a customer record lands.
_TEXT_KINDS = frozenset({"prompt", "assistant", "subagent", "skill"})


def _bounded(value: Any, limit: int, depth: int = 0) -> Any:
    """`value` with every string in it redacted and clipped to `limit`."""
    if isinstance(value, str):
        text, truncated = clip(value, limit)
        return text + "…" if truncated else text
    if value is None or isinstance(value, (bool, int, float)):
        return value
    if isinstance(value, dict) and depth < 2:
        # Keys too: `safe_args` redacts a key and its value as a pair and then emits the
        # key as it found it, so an argument *named* `token=…` carried the value out.
        return {_bounded(str(k), META_CHARS, depth + 1): _bounded(v, limit, depth + 1)
                for k, v in list(value.items())[:16]}
    if isinstance(value, list) and depth < 2:
        return [_bounded(v, limit, depth + 1) for v in value[:16]]
    return f"<{type(value).__name__}>"


def emitted(event: dict) -> dict:
    """An event as it may leave this file. Applied once, in `stream`, to every event.

    Each adapter clips the text it reads, but the names beside it -- a tool, a model, a
    project, a role, a session id -- were copied through as found, so a tool named
    `api_key=…` or a 5,000-character model string reached stdout untouched. Doing it here,
    at the one place events leave, covers every adapter and every field without trusting
    each call site to remember. Underscored fields are internal and never leave.
    """
    out: dict[str, Any] = {}
    kind = event.get("kind")
    for key, value in event.items():
        if key.startswith("_"):
            continue
        if key in _TRUSTED_FIELDS:
            out[key] = value
        elif key in ("text", "truncated"):
            if kind in _TEXT_KINDS:
                out[key] = clip(value, _limits["text"])[0] if key == "text" else value
        elif key == "args":
            # `safe_args` already clipped each value to the budget plus an ellipsis.
            out[key] = _bounded(value, _limits["args"] + 1)
        else:
            out[key] = _bounded(value, META_CHARS)
    return out


# --------------------------------------------------------------------------
# Claude Code.
#
# `~/.claude/projects/<cwd-with-slashes-as-dashes>/<sessionId>.jsonl`, one
# record per line, plus an optional `<sessionId>/subagents/agent-<id>.jsonl`
# holding a subagent's own transcript.
#
# The directory name is a lossy encoding of the working directory -- a real
# hyphen in a path segment is indistinguishable from a separator -- so the
# project is read from the `cwd` field inside the records and the directory
# name is only a fallback.
#
# One logical assistant response is split across several `type: "assistant"`
# lines sharing a `requestId`. Usage is therefore taken from the highest
# `output_tokens` seen for a request rather than summed, which would overcount
# severalfold.
# --------------------------------------------------------------------------


def config_dir(home: Path, variable: str, default: str) -> Path:
    """A provider's configuration directory, honouring its relocation variable.

    The variable applies only to the real home directory: under `--home` a test or a
    copied store is being read, and the operator's own environment must not redirect it.
    """
    moved = os.environ.get(variable)
    if moved and home == Path.home():
        return Path(moved).expanduser()
    return home / default


class ClaudeCodeAdapter(Adapter):
    name = "claude-code"
    root_hint = "~/.claude/projects (or $CLAUDE_CONFIG_DIR/projects)"

    def roots(self, home: Path) -> list[Path]:
        return [config_dir(home, "CLAUDE_CONFIG_DIR", ".claude") / "projects"]

    def discover(self, home: Path, problems: list[Problem]) -> list[SessionRef]:
        refs: list[SessionRef] = []
        for root in self.roots(home):
            if not root.is_dir():
                continue
            # Two depths. A subagent's own turns live only in its sidechain file; the
            # parent records the spawn and the result but not what the subagent did, so
            # globbing one level deep meant the delegated half of every session was
            # invisible -- and the `isSidechain` handling below was unreachable.
            for path in sorted(
                [*root.glob("*/*.jsonl"), *root.glob("*/*/subagents/*.jsonl")]
            ):
                if not path.is_file():
                    problems.append(Problem(self.name, path, "not a regular file"))
                    continue
                if not within(root, path):
                    problems.append(
                        Problem(self.name, path, "resolves outside the store; not followed")
                    )
                    continue
                try:
                    mtime = path.stat().st_mtime
                except OSError as error:
                    problems.append(Problem(self.name, path, f"unreadable: {error.strerror}"))
                    continue
                sidechain = path.parent.name == "subagents"
                refs.append(
                    SessionRef(
                        provider=self.name,
                        path=path,
                        # A subagent file belongs to its parent session, so it carries
                        # the parent's id: its events roll into that session's totals
                        # rather than inventing a session nobody started.
                        session_id=path.parent.parent.name if sidechain else path.stem,
                        project=(path.parent.parent.parent.name if sidechain
                                 else path.parent.name),
                        mtime=mtime,
                        extra={"sidechain": True} if sidechain else None,
                    )
                )
        return refs

    def events(self, ref: SessionRef, problems: list[Problem]) -> Iterator[dict]:
        seq = 0
        seen_uuids: set[str] = set()
        request_tokens: dict[str, int] = {}
        opened = False
        for record in read_jsonl(ref.path, problems, self.name):
            cwd = as_str(record.get("cwd"))
            if cwd and (ref.project or "").startswith("-"):
                ref.project = cwd
            uuid = record.get("uuid")
            if uuid:
                # A resumed session re-serialises earlier records into the new
                # file. Without this, every resume double-counts its history.
                if uuid in seen_uuids:
                    continue
                seen_uuids.add(uuid)
            kind = record.get("type")
            ts = parse_ts(record.get("timestamp"))
            sidechain = bool(record.get("isSidechain"))

            # Open the session on the first conversational record, not the first line.
            # On disk the first line is always bookkeeping -- `queue-operation`,
            # `last-prompt`, `mode` -- which carries neither a working directory nor a
            # version, so every session-open event came out with both null.
            if not opened and kind in ("user", "assistant"):
                opened = True
                seq += 1
                yield base_event(
                    ref, seq, "session", ts, cwd=cwd, agent=as_str(record.get("version"))
                )

            if kind == "user":
                message = record.get("message") or {}
                content = message.get("content")
                blocks = content if isinstance(content, list) else [{"type": "text", "text": content}]
                first = blocks[0] if blocks and isinstance(blocks[0], dict) else {}
                if first.get("type") == "tool_result":
                    result = record.get("toolUseResult")
                    ok = not first.get("is_error")
                    seq += 1
                    yield base_event(
                        ref, seq, "tool_result", ts,
                        ok=ok,
                        bytes=_rough_len(first.get("content")),
                        agent_id=result.get("agentId") if isinstance(result, dict) else None,
                        _id=as_str(first.get("tool_use_id")),
                    )
                    continue
                # `isMeta` marks an injected turn -- a hook's output, a system
                # banner -- presented as a user message. It is not something
                # the person typed, and counting it as a prompt inflates every
                # frequency an audit reports.
                if record.get("isMeta") or record.get("isCompactSummary") or sidechain:
                    continue
                # `origin.kind` says who a user turn came from. Anything other than a
                # person -- a background task's completion notice, a message from a peer
                # session, a coordinator -- is a user record on the wire and not a
                # prompt. On real data those were 40% of what this counted as prompts.
                origin = record.get("origin")
                if isinstance(origin, dict) and origin.get("kind") not in (None, "human"):
                    continue
                raw = _text_of(blocks)
                # A slash command is the user choosing a procedure by name, which is a
                # routing decision, and it arrives as a `<command-name>` wrapper.
                command = _COMMAND.search(raw)
                if command:
                    seq += 1
                    yield base_event(
                        ref, seq, "skill", ts, skill=command.group(1).lstrip("/"),
                        text=clip(_COMMAND_ARGS.search(raw).group(1), _limits["text"])[0]
                        if _COMMAND_ARGS.search(raw) else None,
                        via="slash-command",
                    )
                    continue
                text, truncated = clip(typed_text(raw), _limits["text"])
                if text:
                    seq += 1
                    yield base_event(ref, seq, "prompt", ts, text=text, truncated=truncated or None)

            elif kind == "assistant":
                message = record.get("message") or {}
                model = message.get("model")
                usage = message.get("usage") or {}
                request_id = record.get("requestId") or message.get("id")
                if request_id and isinstance(usage, dict):
                    # Guarded: one malformed count used to raise, and the stream's catch-all
                    # then discarded every later event in the file.
                    try:
                        tokens = int(usage.get("output_tokens") or 0)
                    except (TypeError, ValueError):
                        tokens = 0
                    request_tokens[request_id] = max(request_tokens.get(request_id, 0), tokens)
                for block in message.get("content") or []:
                    if not isinstance(block, dict):
                        continue
                    if block.get("type") == "text":
                        text, truncated = clip(block.get("text"), _limits["text"])
                        if text:
                            seq += 1
                            yield base_event(
                                ref, seq, "assistant", ts, text=text,
                                truncated=truncated or None, model=model,
                                sidechain=sidechain or None,
                            )
                    elif block.get("type") == "tool_use":
                        seq += 1
                        yield from _tool_use_events(
                            ref, seq, ts, block, model=model, sidechain=sidechain
                        )
        if request_tokens:
            seq += 1
            yield base_event(
                ref, seq, "session", None,
                usage={"output_tokens": sum(request_tokens.values()), "requests": len(request_tokens)},
                # Per request, so `stream` can drop the requests a resumed or forked file
                # replays. The `uuid` check above only sees within one file.
                _requests=request_tokens,
            )


def _codex_text(content: Any) -> str:
    """Join the text of a Codex `response_item/message` content list."""
    if isinstance(content, str):
        return content
    parts = []
    for block in content or []:
        if isinstance(block, dict) and isinstance(block.get("text"), str):
            parts.append(block["text"])
        elif isinstance(block, str):
            parts.append(block)
    return " ".join(p for p in parts if p)


def _rough_len(value: Any) -> int:
    if isinstance(value, str):
        return len(value)
    try:
        return len(json.dumps(value))
    except (TypeError, ValueError):
        return 0


def _text_of(blocks: Any) -> str:
    """Join the text of a content list, across the shapes providers actually use.

    A block is text if it *says* it is text, or if it simply carries a `text` string
    and claims to be nothing else -- which is Gemini's native part shape. Requiring
    `type == "text"` dropped every Gemini prompt on disk while reporting no problem.
    """
    if isinstance(blocks, str):
        return blocks
    if isinstance(blocks, dict):
        # Iterating a dict yields its keys, so `{"content": [...]}` came out as the
        # literal word "content" -- for every turn of every Cursor agent transcript.
        return _text_of(blocks.get("content", blocks.get("text")))
    if not isinstance(blocks, list):
        return ""
    parts = []
    for block in blocks:
        if isinstance(block, str):
            parts.append(block)
        elif isinstance(block, dict) and isinstance(block.get("text"), str):
            if block.get("type") in (None, "text", "input_text", "output_text"):
                parts.append(block["text"])
    return " ".join(p for p in parts if p)


_COMMAND = re.compile(r"<command-name>\s*([^<\s]+)\s*</command-name>")
_COMMAND_ARGS = re.compile(r"<command-args>(.*?)</command-args>", re.S)


def _tool_use_events(
    ref: SessionRef, seq: int, ts: datetime | None, block: dict, *, model=None, sidechain=False
) -> Iterator[dict]:
    """Emit the right kind for a tool_use block.

    A subagent spawn and a skill invocation are ordinary tool calls on the
    wire, and they are the two an audit cares about most, so they get their own
    kinds rather than being buried in a `Task`/`Agent` row of a histogram.
    """
    name = as_str(block.get("name")) or "?"
    args = block.get("input") if isinstance(block.get("input"), dict) else {}
    # `Task` was renamed `Agent`; sessions on disk carry both.
    if name in ("Task", "Agent"):
        text, truncated = clip(args.get("description") or args.get("prompt"), _limits["text"])
        yield base_event(
            ref, seq, "subagent", ts,
            agent_type=as_str(args.get("subagent_type")) or "unspecified",
            text=text or None, truncated=truncated or None, tool=name,
        )
        return
    if name == "Skill" and args.get("skill"):
        text, truncated = clip(args.get("args"), _limits["text"])
        yield base_event(ref, seq, "skill", ts, skill=str(args.get("skill")), text=text or None)
        return
    yield base_event(
        ref, seq, "tool_call", ts, tool=name,
        args=safe_args(args, _limits["args"]), model=model, sidechain=sidechain or None,
        _id=as_str(block.get("id")),
    )


# --------------------------------------------------------------------------
# OpenAI Codex CLI.
#
# `~/.codex/sessions/YYYY/MM/DD/rollout-<timestamp>-<uuid>.jsonl`. Each line is
# `{timestamp, type, payload}`; the meaningful discriminator is the pair
# `(type, payload.type)`, because `response_item` and `event_msg` both carry
# several shapes. Older files predate the envelope entirely and simply yield
# nothing, which is the intended degradation.
#
# `function_call.arguments` is JSON inside a string, and
# `function_call_output.output` is free text with a build-specific header. Both
# are treated as opaque.
# --------------------------------------------------------------------------


class CodexAdapter(Adapter):
    name = "codex"
    root_hint = "~/.codex/sessions (or $CODEX_HOME/sessions)"

    def roots(self, home: Path) -> list[Path]:
        return [config_dir(home, "CODEX_HOME", ".codex") / "sessions"]

    def discover(self, home: Path, problems: list[Problem]) -> list[SessionRef]:
        refs: list[SessionRef] = []
        for root in self.roots(home):
            if not root.is_dir():
                continue
            for path in sorted(root.glob("*/*/*/rollout-*.jsonl")):
                if not path.is_file():
                    problems.append(Problem(self.name, path, "not a regular file"))
                    continue
                if not within(root, path):
                    problems.append(
                        Problem(self.name, path, "resolves outside the store; not followed")
                    )
                    continue
                try:
                    mtime = path.stat().st_mtime
                except OSError as error:
                    problems.append(Problem(self.name, path, f"unreadable: {error.strerror}"))
                    continue
                header = self._header_of(path)
                parent = header.get("parent")
                refs.append(
                    SessionRef(
                        provider=self.name, path=path,
                        # A subagent rollout carries its parent's id from discovery on, not
                        # only once `events` has parsed it. The session cap is applied
                        # between the two, and there each child counted as a session of
                        # its own: under `--max-sessions 1` a parent with three newer
                        # children was dropped in favour of one of them.
                        session_id=parent or header.get("id") or uuid_tail(path.stem),
                        project=header.get("cwd"),
                        mtime=mtime,
                        extra={"sidechain": True} if parent else None,
                    )
                )
        return refs

    @staticmethod
    def _header_of(path: Path) -> dict[str, str | None]:
        """Read the working directory, session id and parent id out of the first line only.

        Codex stores the project inside the file, not in the path, so without
        this a `--project` filter silently discarded every Codex session --
        the worst shape of failure here, because the report would then say it
        examined Codex and found nothing. One line per file keeps discovery
        cheap over a store with tens of thousands of rollouts.
        """
        try:
            with path.open("rb") as handle:
                # Bounded like every other line: an unbounded `readline` here was a
                # whole-file read for a rollout with no newline in it.
                first = handle.readline(MAX_LINE_BYTES + 1)
        except OSError:
            return {}
        if len(first) > MAX_LINE_BYTES and not first.endswith(b"\n"):
            return {}  # `events` reports the oversized line.
        try:
            record = json.loads(first.decode("utf-8-sig", errors="replace"))
        except (ValueError, RecursionError):
            return {}
        payload = record.get("payload") if isinstance(record, dict) else None
        if not isinstance(payload, dict):
            return {}
        meta = record.get("type") == "session_meta"
        return {
            "cwd": as_str(payload.get("cwd")),
            # The same fields, read the same way, as `events` reads from the same line.
            "id": as_str(payload.get("id") or payload.get("session_id")) if meta else None,
            "parent": as_str(_dig(payload.get("source"), "subagent", "thread_spawn",
                                  "parent_thread_id")) if meta else None,
        }

    def events(self, ref: SessionRef, problems: list[Problem]) -> Iterator[dict]:
        seq = 0
        calls: dict[str, str] = {}
        enveloped = False
        header_seen = False
        model: str | None = None
        # `token_count` carries a *running* total for the session, emitted after every
        # turn -- up to 1,688 times in one file on real data. Summing them overcounted
        # usage roughly 26x. Only the last one is the session's usage.
        last_usage: dict[str, Any] | None = None
        # The same turn is recorded more than once: as `event_msg/user_message` and as
        # `response_item/message`, in every version on disk. Counting both doubled every
        # prompt. Texts already emitted in the current turn are skipped; a person who
        # genuinely types the same thing in a later turn is still counted.
        turn_texts: set[tuple[str, str]] = set()
        # A spawn is recorded as the `collaboration.spawn_agent` call and again, on
        # builds from 0.147, as a `SubAgentActivity` item with kind "started" -- one for
        # one. The call is the decision, so it is what counts; a "started" record is
        # counted only when no call preceded it, which covers builds that lack the call.
        pending_spawns = 0
        mcp_seen: set[str] = set()
        # In a subagent's rollout the "user" turns were written by the parent agent --
        # its task and its follow-ups. They are already represented by the spawn in the
        # parent, and counting them as prompts made two or three sibling subagents,
        # spawned together with the same instructions, look like a person typing the
        # same thing three times.
        sidechain = False

        def once(role: str, text: str) -> bool:
            key = (role, " ".join(text.split())[:400])
            if key in turn_texts:
                return False
            turn_texts.add(key)
            return True

        for record in read_jsonl(ref.path, problems, self.name):
            outer = record.get("type")
            payload = record.get("payload")
            if not isinstance(payload, dict):
                continue
            enveloped = True
            ts = parse_ts(record.get("timestamp"))
            inner = payload.get("type")

            if outer == "session_meta":
                # Only the first. A forked rollout replays its parent's header on line 2,
                # and taking that one moved every later event under the parent's id and
                # left a one-event phantom session behind.
                if header_seen:
                    continue
                header_seen = True
                ref.project = as_str(payload.get("cwd")) or ref.project
                ref.session_id = as_str(payload.get("id") or payload.get("session_id")) or ref.session_id
                # A subagent's rollout is its own file but not its own piece of work: it
                # carries the parent's id, the way a Claude Code sidechain file does, so
                # a session that delegated eleven times is one session and not twelve.
                parent = as_str(_dig(payload.get("source"), "subagent", "thread_spawn",
                                     "parent_thread_id"))
                if parent:
                    ref.session_id = parent
                    sidechain = True
                seq += 1
                yield base_event(
                    ref, seq, "session", ts,
                    cwd=as_str(payload.get("cwd")), agent=as_str(payload.get("cli_version")),
                    sidechain=True if parent else None,
                )
            elif outer == "turn_context":
                ref.project = as_str(payload.get("cwd")) or ref.project
                model = as_str(payload.get("model")) or model
                turn_texts.clear()
            elif (outer == "response_item" and inner == "message") or (
                outer == "event_msg" and inner in ("user_message", "agent_message")
            ):
                if outer == "event_msg":
                    role = "user" if inner == "user_message" else "assistant"
                    raw = payload.get("message")
                    raw = raw if isinstance(raw, str) else ""
                else:
                    role = as_str(payload.get("role")) or ""
                    raw = _codex_text(payload.get("content"))
                if role not in ("user", "assistant"):
                    continue  # `developer` turns are injected instructions.
                if role == "user" and sidechain:
                    continue
                body = typed_text(raw) if role == "user" else raw
                text, truncated = clip(body, _limits["text"])
                if text and once(role, body):
                    seq += 1
                    yield base_event(
                        ref, seq, "prompt" if role == "user" else "assistant", ts,
                        text=text, truncated=truncated or None,
                        model=model if role == "assistant" else None,
                        sidechain=sidechain or None,
                    )
            elif outer == "event_msg" and inner == "sub_agent_activity":
                # 0.144-0.150. The payload names an `agent_path`, not an agent type.
                if payload.get("kind") in (None, "started"):
                    if pending_spawns:
                        pending_spawns -= 1
                    else:
                        seq += 1
                        yield base_event(ref, seq, "subagent", ts,
                                         agent_type=_agent_label(payload))
            elif outer == "event_msg" and inner == "item_completed":
                item = payload.get("item") if isinstance(payload.get("item"), dict) else {}
                item_type = item.get("type")
                if item_type == "SubAgentActivity" and item.get("kind") == "started":
                    if pending_spawns:
                        pending_spawns -= 1
                    else:
                        seq += 1
                        yield base_event(ref, seq, "subagent", ts, agent_type=_agent_label(item))
                elif item_type == "McpToolCall":
                    # MCP calls exist only here (and, on 0.147-0.148, as
                    # `mcp_tool_call_end`). There is no `function_call` for them, so
                    # without this every MCP tool the agent used was invisible.
                    key = as_str(item.get("id")) or ""
                    if key and key in mcp_seen:
                        continue
                    mcp_seen.add(key)
                    seq += 1
                    yield base_event(
                        ref, seq, "tool_call", ts,
                        tool=_mcp_name(item.get("server"), item.get("tool")),
                        args=safe_args(item.get("arguments"), _limits["args"]),
                        ok=as_str(item.get("status")) in (None, "completed", "success"),
                        _id=key or None,
                    )
                # Every other item type restates a record already handled above --
                # messages, reasoning, command executions -- and is ignored so it is not
                # counted twice.
            elif outer == "event_msg" and inner == "mcp_tool_call_end":
                invocation = payload.get("invocation") if isinstance(payload.get("invocation"), dict) else {}
                key = as_str(payload.get("call_id")) or ""
                if key and key in mcp_seen:
                    continue
                mcp_seen.add(key)
                result = payload.get("result")
                seq += 1
                yield base_event(
                    ref, seq, "tool_call", ts,
                    tool=_mcp_name(invocation.get("server"), invocation.get("tool")),
                    args=safe_args(invocation.get("arguments"), _limits["args"]),
                    ok=not (isinstance(result, dict) and "Err" in result),
                    _id=key or None,
                )
            elif outer == "compacted" or (outer == "event_msg" and inner == "context_compacted"):
                seq += 1
                yield base_event(ref, seq, "session", ts, compacted=True)
            elif outer == "event_msg" and inner == "token_count":
                info = payload.get("info")
                total = info.get("total_token_usage") if isinstance(info, dict) else None
                if isinstance(total, dict):
                    last_usage = total
            elif outer == "response_item" and inner in ("function_call", "custom_tool_call"):
                name = as_str(payload.get("name")) or "?"
                # Tools are namespaced (`collaboration.spawn_agent`, `clock.sleep`), and
                # dropping the namespace merged unrelated tools under one bare name.
                namespace = as_str(payload.get("namespace"))
                qualified = f"{namespace}.{name}" if namespace else name
                call_id = payload.get("call_id")
                if call_id:
                    calls[str(call_id)] = qualified
                raw = payload.get("arguments", payload.get("input"))
                if isinstance(raw, str):
                    try:
                        raw = json.loads(raw)
                    except (ValueError, RecursionError):
                        pass
                seq += 1
                if qualified == "collaboration.spawn_agent":
                    pending_spawns += 1
                    args = raw if isinstance(raw, dict) else {}
                    text, truncated = clip(args.get("task_name") or args.get("message"),
                                           _limits["text"])
                    yield base_event(
                        ref, seq, "subagent", ts, tool=qualified,
                        agent_type=as_str(args.get("agent_type") or args.get("role"))
                        or "unspecified",
                        text=text or None, truncated=truncated or None,
                    )
                else:
                    yield base_event(
                        ref, seq, "tool_call", ts, tool=qualified,
                        args=safe_args(raw, _limits["args"]), model=model,
                        _id=as_str(call_id),
                    )
            elif outer == "response_item" and inner in (
                "function_call_output", "custom_tool_call_output"
            ):
                call_id = str(payload.get("call_id") or "")
                seq += 1
                yield base_event(
                    ref, seq, "tool_result", ts,
                    tool=calls.get(call_id), ok=True, bytes=_rough_len(payload.get("output")),
                    _id=call_id or None,
                )
        if last_usage is not None:
            seq += 1
            yield base_event(
                ref, seq, "session", None,
                usage={k: last_usage.get(k) for k in ("input_tokens", "output_tokens")
                       if isinstance(last_usage.get(k), (int, float))},
            )
        if not enveloped and ref.path.stat().st_size:
            # Rollouts written before the `{type, payload}` envelope existed parse
            # cleanly and mean nothing to this mapping. Saying so is the whole point:
            # a file that yields no events and no problem is indistinguishable from a
            # session in which nothing happened.
            problems.append(
                Problem(self.name, ref.path, "no {type, payload} envelope; too old to parse")
            )


def _dig(value: Any, *keys: str) -> Any:
    for key in keys:
        if not isinstance(value, dict):
            return None
        value = value.get(key)
    return value


def _agent_label(payload: dict) -> str:
    """A Codex subagent's label: the last segment of its `agent_path`."""
    path = as_str(payload.get("agent_path")) or as_str(payload.get("agent")) or ""
    return path.rstrip("/").rsplit("/", 1)[-1] or "unspecified"


def _mcp_name(server: Any, tool: Any) -> str:
    return f"mcp__{as_str(server) or '?'}__{as_str(tool) or '?'}"


# --------------------------------------------------------------------------
# Gemini CLI.
#
# `~/.gemini/tmp/<project>/chats/session-*.jsonl`. A `kind: "main"` header line
# carries the session id and start time; `user` and `gemini` lines carry the
# turns. `$set` lines are incremental updates to earlier records and are
# ignored: replaying them would double-count turns.
# --------------------------------------------------------------------------


class GeminiCliAdapter(Adapter):
    name = "gemini-cli"
    root_hint = "~/.gemini/tmp/*/chats"

    def roots(self, home: Path) -> list[Path]:
        return [home / ".gemini" / "tmp"]

    def discover(self, home: Path, problems: list[Problem]) -> list[SessionRef]:
        refs: list[SessionRef] = []
        for root in self.roots(home):
            if not root.is_dir():
                continue
            for path in sorted(root.glob("*/chats/*")):
                if not path.is_file():
                    problems.append(Problem(self.name, path, "not a regular file"))
                    continue
                if not within(root, path):
                    problems.append(
                        Problem(self.name, path, "resolves outside the store; not followed")
                    )
                    continue
                try:
                    mtime = path.stat().st_mtime
                except OSError as error:
                    problems.append(Problem(self.name, path, f"unreadable: {error.strerror}"))
                    continue
                refs.append(
                    SessionRef(
                        provider=self.name, path=path, session_id=path.stem,
                        project=path.parent.parent.name, mtime=mtime,
                    )
                )
        return refs

    def events(self, ref: SessionRef, problems: list[Problem]) -> Iterator[dict]:
        # Two formats live side by side in the same directory. A `.json` file is one
        # pretty-printed document with a `messages` array, and reading it a line at a
        # time produced thousands of "unparsable line" problems and no events -- while
        # `discover` still counted it as a session, so the totals looked healthy.
        if ref.path.suffix.lower() == ".json":
            document = read_json(ref.path, problems, self.name)
            records = document.get("messages") if isinstance(document, dict) else None
            if isinstance(document, dict):
                header = {k: document[k] for k in ("sessionId", "projectHash", "startTime")
                          if k in document}
                header["kind"] = "main"
                records = [header] + list(records or [])
            source: Iterable[dict] = [r for r in (records or []) if isinstance(r, dict)]
        else:
            source = read_jsonl(ref.path, problems, self.name)
        seq = 0
        for record in source:
            if "$set" in record:
                continue
            ts = parse_ts(record.get("timestamp") or record.get("startTime"))
            if record.get("kind") == "main":
                ref.session_id = str(record.get("sessionId") or ref.session_id)
                seq += 1
                yield base_event(ref, seq, "session", ts)
                continue
            kind = record.get("type")
            if kind == "user":
                text, truncated = clip(
                    _content_text(record.get("displayContent"))
                    or _text_of(record.get("content")),
                    _limits["text"],
                )
                if text:
                    seq += 1
                    yield base_event(ref, seq, "prompt", ts, text=text, truncated=truncated or None)
            elif kind == "gemini":
                tokens = record.get("tokens") if isinstance(record.get("tokens"), dict) else None
                text, truncated = clip(record.get("content"), _limits["text"])
                if text:
                    seq += 1
                    yield base_event(
                        ref, seq, "assistant", ts, text=text, truncated=truncated or None,
                        model=record.get("model"), usage=tokens,
                    )
                for call in record.get("toolCalls") or []:
                    if not isinstance(call, dict):
                        continue
                    seq += 1
                    status = as_str(call.get("status"))
                    yield base_event(
                        ref, seq, "tool_call", ts,
                        tool=as_str(call.get("name") or call.get("tool")) or "?",
                        args=safe_args(call.get("args") or call.get("arguments"), _limits["args"]),
                        ok=None if status is None else status not in ("error", "cancelled"),
                    )


# --------------------------------------------------------------------------
# Cline.
#
# `~/.cline/data/sessions/<id>/<id>.messages.json` holds the turns;
# `<id>.json` beside it holds the metadata, including the working directory.
# --------------------------------------------------------------------------


class ClineAdapter(Adapter):
    name = "cline"
    root_hint = "~/.cline/data/sessions"

    def roots(self, home: Path) -> list[Path]:
        return [home / ".cline" / "data" / "sessions"]

    def discover(self, home: Path, problems: list[Problem]) -> list[SessionRef]:
        refs: list[SessionRef] = []
        for root in self.roots(home):
            if not root.is_dir():
                continue
            for path in sorted(root.glob("*/*.messages.json")):
                if not path.is_file():
                    problems.append(Problem(self.name, path, "not a regular file"))
                    continue
                if not within(root, path):
                    problems.append(
                        Problem(self.name, path, "resolves outside the store; not followed")
                    )
                    continue
                try:
                    mtime = path.stat().st_mtime
                except OSError as error:
                    problems.append(Problem(self.name, path, f"unreadable: {error.strerror}"))
                    continue
                meta_path = path.with_name(path.name.replace(".messages.json", ".json"))
                project = None
                # The metadata file gets the same guards as the transcript beside it. It
                # was followed through a symlink to wherever it pointed, and a `cwd` that
                # was an object became a set member in `discover` and crashed the run.
                if meta_path.is_file() and not within(root, meta_path):
                    problems.append(
                        Problem(self.name, meta_path, "resolves outside the store; not followed")
                    )
                elif meta_path.is_file():
                    meta = read_json(meta_path, problems, self.name)
                    if isinstance(meta, dict):
                        project = as_str(meta.get("cwd"))
                refs.append(
                    SessionRef(
                        provider=self.name, path=path,
                        session_id=path.name.replace(".messages.json", ""),
                        project=project, mtime=mtime,
                    )
                )
        return refs

    def events(self, ref: SessionRef, problems: list[Problem]) -> Iterator[dict]:
        document = read_json(ref.path, problems, self.name)
        if not isinstance(document, dict):
            return
        seq = 1
        yield base_event(ref, seq, "session", parse_ts(document.get("updated_at")))
        for message in document.get("messages") or []:
            if not isinstance(message, dict):
                continue
            ts = parse_ts(message.get("ts"))
            role = message.get("role")
            model = (message.get("modelInfo") or {}).get("id") if isinstance(
                message.get("modelInfo"), dict
            ) else None
            text, truncated = clip(_content_text(message.get("content")), _limits["text"])
            for call in _content_tool_calls(message.get("content")):
                seq += 1
                yield base_event(
                    ref, seq, "tool_call", ts,
                    tool=as_str(call.get("name")) or "?",
                    args=safe_args(call.get("input") or call.get("args"), _limits["args"]),
                )
            for result in _content_blocks(message.get("content"), "tool_result"):
                seq += 1
                yield base_event(
                    ref, seq, "tool_result", ts,
                    ok=not result.get("is_error"), bytes=_rough_len(result.get("content")),
                )
            if not text:
                continue
            seq += 1
            if role == "user":
                yield base_event(ref, seq, "prompt", ts, text=text, truncated=truncated or None)
            else:
                yield base_event(
                    ref, seq, "assistant", ts, text=text, truncated=truncated or None, model=model
                )


def _content_text(content: Any) -> str:
    if isinstance(content, str):
        return content
    return _text_of(content)


def _content_blocks(content: Any, kind: str) -> list[dict]:
    if not isinstance(content, list):
        return []
    return [b for b in content if isinstance(b, dict) and b.get("type") == kind]


def _content_tool_calls(content: Any) -> list[dict]:
    if not isinstance(content, list):
        return []
    return [
        block for block in content
        if isinstance(block, dict) and block.get("type") in ("tool_use", "tool_call")
    ]


# --------------------------------------------------------------------------
# Generic export.
#
# For a provider with no local store, or one whose format is not supported: the
# user exports a conversation and points this at the file. Anything it cannot
# classify is emitted as `kind: "unknown"` rather than guessed into a role,
# because a misattributed turn is worse for a frequency count than an
# unattributed one.
# --------------------------------------------------------------------------

# `## User`, `**Assistant:**`, `> Human:`, `- AI —`. The colon sits inside the bold
# delimiters at least as often as outside, and an export using that convention was being
# absorbed into one undifferentiated blob instead of split into turns.
_MD_SPEAKER = re.compile(
    r"^(?:[>\-*+]\s*)?(?:#{1,6}\s*)?(?:\*\*|__)?"
    r"(user|human|you|assistant|agent|ai|claude|codex|model)"
    r"\s*[:\-\u2014]?\s*(?:\*\*|__)?\s*[:\-\u2014]?\s*$",
    re.I,
)
_ROLE_MAP = {
    "user": "prompt", "human": "prompt", "you": "prompt",
    "assistant": "assistant", "agent": "assistant", "ai": "assistant",
    "claude": "assistant", "codex": "assistant", "model": "assistant",
}
_CALL_ROLES = ("tool_call", "function_call", "tool_use")
_RESULT_ROLES = (
    "tool_result", "tool_output", "function_result", "function_call_output",
    "custom_tool_call_output",
)
# A role with one of these endings is output, whatever comes before it:
# `function_output`, `tool_response`, `lookup_result`. Listing names alone let every
# spelling not on the list through as an `unknown` turn with the output as its text.
_RESULT_SUFFIXES = tuple(f"{sep}{word}" for sep in "_-" for word in ("output", "result", "response"))


class GenericAdapter(Adapter):
    name = "generic"
    root_hint = "(a file you name with --input)"

    def roots(self, home: Path) -> list[Path]:
        return []

    def discover(self, home: Path, problems: list[Problem]) -> list[SessionRef]:
        return []

    def refs_for(self, paths: Iterable[Path], problems: list[Problem]) -> list[SessionRef]:
        refs = []
        for path in paths:
            if not path.is_file():
                problems.append(Problem(self.name, path, "not a file"))
                continue
            refs.append(
                SessionRef(
                    provider=self.name, path=path, session_id=path.name,
                    project=str(path.parent), mtime=path.stat().st_mtime,
                )
            )
        return refs

    def events(self, ref: SessionRef, problems: list[Problem]) -> Iterator[dict]:
        suffix = ref.path.suffix.lower()
        if suffix == ".jsonl":
            yield from self._records(ref, read_jsonl(ref.path, problems, self.name))
        elif suffix == ".json":
            document = read_json(ref.path, problems, self.name)
            yield from self._records(ref, _iter_records(document, problems, ref.path))
        else:
            yield from self._prose(ref, problems)

    def _records(self, ref: SessionRef, records: Iterable[Any]) -> Iterator[dict]:
        seq = 1
        yield base_event(ref, seq, "session", None)
        for record in records:
            if not isinstance(record, dict):
                continue
            seq += 1
            ts = parse_ts(
                record.get("timestamp") or record.get("ts") or record.get("time")
                or record.get("createdAt") or record.get("created_at")
            )
            role = (as_str(record.get("role") or record.get("type") or record.get("sender"))
                    or "").lower()
            tool = as_str(record.get("tool") or record.get("name") or record.get("tool_name"))
            body = (
                record.get("content") if "content" in record
                else record.get("text", record.get("message"))
            )
            # The Anthropic-message shape nests the turn: `{role, message: {content}}`.
            # It is what Cursor's agent CLI writes and what an exported Claude session
            # looks like, and both used to come out as the literal word "content".
            if isinstance(body, dict):
                body = body.get("content", body.get("text"))
            # A tool's output, in the shapes an export uses for it: OpenAI's
            # `{role: "tool", tool_call_id, content}`, the legacy `role: "function"`, an
            # explicit `tool_result`. It was emitted as `kind: unknown` with the output as
            # its text -- the one field the privacy contract says is never reproduced. It
            # is reduced to success and size here, as every other adapter does. A `tool` or
            # `function` record that names a tool and carries arguments rather than output
            # is still a call, which is how the partner-triage export writes one.
            #
            # A role that merely *mentions* a tool or function -- `tool`, `function`,
            # `tool_message` -- is a call when it names one and carries arguments, and
            # output otherwise. One with an output ending is always output.
            arguments = record.get("args") or record.get("arguments") or record.get("input")
            toolish = "tool" in role or "function" in role
            if role not in _CALL_ROLES and (
                role in _RESULT_ROLES
                or role.endswith(_RESULT_SUFFIXES)
                or "tool_call_id" in record
                or (toolish and (not tool or arguments is None))
            ):
                output = body if body is not None else record.get("output")
                yield base_event(
                    ref, seq, "tool_result", ts, tool=tool,
                    ok=not record.get("is_error"), bytes=_rough_len(output),
                )
                continue
            calls = _content_tool_calls(body)
            for call in calls:
                seq += 1
                yield base_event(
                    ref, seq, "tool_call", ts, tool=as_str(call.get("name")) or "?",
                    args=safe_args(call.get("input") or call.get("args"), _limits["args"]),
                )
            kind = _ROLE_MAP.get(role)
            raw = _content_text(body)
            if calls and not raw:
                continue
            text, truncated = clip(typed_text(raw) if kind == "prompt" else raw,
                                   _limits["text"])
            if kind == "prompt" and not text:
                continue
            if tool and (toolish or role in _CALL_ROLES or not kind):
                yield base_event(
                    ref, seq, "tool_call", ts, tool=str(tool),
                    args=safe_args(arguments, _limits["args"]),
                )
                continue
            if not kind:
                # Counted, with its role and size, but not quoted. A role this file does
                # not recognise is as likely to be tool output under an unfamiliar name
                # (`observation`, `ipython`, `environment`) as a turn, and quoting it
                # would break the one rule the privacy contract states outright. Turns a
                # person or an agent wrote are recognised by role and still come through.
                yield base_event(ref, seq, "unknown", ts, role=role or None,
                                 bytes=_rough_len(body))
                continue
            yield base_event(ref, seq, kind, ts, text=text or None, truncated=truncated or None)

    def _prose(self, ref: SessionRef, problems: list[Problem]) -> Iterator[dict]:
        """Split a Markdown or plain-text log on speaker headings.

        Only a heading that is *nothing but* a speaker name counts. A sentence
        beginning "Assistant: ..." mid-paragraph is prose, and treating it as a
        turn boundary would shred a transcript into fragments and report them
        as frequency.
        """
        content = read_bounded(ref.path, problems, self.name)
        if content is None:
            return
        lines = content.splitlines()
        seq = 1
        yield base_event(ref, seq, "session", None)
        kind = "unknown"
        buffer: list[str] = []
        for line in lines + ["\u0000"]:
            match = _MD_SPEAKER.match(line.strip())
            if match or line == "\u0000":
                if buffer:
                    body = " ".join(buffer)
                    text, truncated = clip(body, _limits["text"])
                    if text and kind == "unknown":
                        # Text before the first speaker heading. Counted, not quoted, for
                        # the reason given in `_records`.
                        seq += 1
                        yield base_event(ref, seq, kind, None, bytes=len(body.strip()))
                    elif text:
                        seq += 1
                        yield base_event(
                            ref, seq, kind, None, text=text, truncated=truncated or None
                        )
                    buffer = []
                if match:
                    kind = _ROLE_MAP.get(match.group(1).lower(), "unknown")
            else:
                buffer.append(line)


def _iter_records(document: Any, problems: list[Problem] | None = None,
                  path: Any = "-") -> Iterable[Any]:
    if isinstance(document, list):
        return document
    if isinstance(document, dict):
        for key in ("messages", "requests", "conversation", "turns", "events", "history"):
            value = document.get(key)
            if isinstance(value, list):
                return value
    # Saying nothing here made a document of an unrecognised shape look like an empty
    # conversation.
    if problems is not None and document is not None:
        problems.append(Problem("generic", path, "no list of turns found in this document"))
    return []


# --------------------------------------------------------------------------
# Cursor agent CLI.
#
# `~/.cursor/projects/<project>/agent-transcripts/<uuid>/<file>.jsonl`, one
# `{role, message: {content: [...]}}` record per line -- the Anthropic message
# shape, so the generic record reader parses it. This is the terminal agent, not
# the editor: the editor's chats live in SQLite and remain unparsed.
# --------------------------------------------------------------------------


class CursorAgentAdapter(GenericAdapter):
    name = "cursor-agent"
    root_hint = "~/.cursor/projects/*/agent-transcripts"

    def roots(self, home: Path) -> list[Path]:
        return [home / ".cursor" / "projects"]

    def discover(self, home: Path, problems: list[Problem]) -> list[SessionRef]:
        refs: list[SessionRef] = []
        for root in self.roots(home):
            if not root.is_dir():
                continue
            for path in sorted(root.glob("*/agent-transcripts/*/*.jsonl")):
                if not path.is_file():
                    problems.append(Problem(self.name, path, "not a regular file"))
                    continue
                if not within(root, path):
                    problems.append(
                        Problem(self.name, path, "resolves outside the store; not followed")
                    )
                    continue
                try:
                    mtime = path.stat().st_mtime
                except OSError as error:
                    problems.append(Problem(self.name, path, f"unreadable: {error.strerror}"))
                    continue
                refs.append(
                    SessionRef(
                        provider=self.name, path=path, session_id=path.parent.name,
                        project=path.parent.parent.parent.name, mtime=mtime,
                    )
                )
        return refs

    def events(self, ref: SessionRef, problems: list[Problem]) -> Iterator[dict]:
        yield from self._records(ref, read_jsonl(ref.path, problems, self.name))


ADAPTERS: dict[str, Adapter] = {
    adapter.name: adapter
    for adapter in (
        ClaudeCodeAdapter(), CodexAdapter(), GeminiCliAdapter(), ClineAdapter(),
        CursorAgentAdapter(), GenericAdapter(),
    )
}

# Providers with a known local store this script deliberately does not parse.
# Listed so `discover` can say why rather than leaving a user to conclude their
# history was not found. See references/providers.md.
UNSUPPORTED = {
    "cursor": "~/.config/Cursor/User/globalStorage/state.vscdb and ~/.cursor/chats/*/*/store.db "
              "(the editor's chats: SQLite, no stability contract)",
    "copilot-chat": "~/.config/Code/User/workspaceStorage/*/chatSessions/*.json",
    "copilot-cli": "~/.copilot/session-state/*/events.jsonl",
    "grok-cli": "~/.grok/session_search.sqlite",
    "antigravity-cli": "~/.gemini/antigravity-cli/conversations/*.pb, *.db (protobuf, SQLite)",
    "opencode": "~/.local/share/opencode/opencode.db (SQLite)",
    "pi": "~/.pi/agent/sessions/*/*.jsonl",
}

_limits = {"text": DEFAULT_MAX_CHARS, "args": DEFAULT_ARG_CHARS}


# --------------------------------------------------------------------------
# Selection.
# --------------------------------------------------------------------------


def select(args: argparse.Namespace) -> tuple[list[SessionRef], list[Problem], dict[str, Any]]:
    problems: list[Problem] = []
    home = Path(args.home).expanduser() if args.home else Path.home()
    # `--since` and `--until` arrive parsed: `_moment` rejected anything else, where an
    # unparsable date once fell back to the default window without a word.
    until: datetime | None = args.until
    refs: list[SessionRef] = []
    project_filter = args.project

    if args.input:
        # A file the user named is read under the window they asked for, and only that
        # one. The default window is a budget for *discovery*; applied here it would hide
        # an export made two months ago, which the user pointed at on purpose. An export's
        # mtime says when it was exported, so the window is applied to the events inside
        # it (in `stream`), never to the file.
        refs = ADAPTERS["generic"].refs_for([Path(p).expanduser() for p in args.input],
                                            problems)
        since = args.since or (datetime.now(timezone.utc) - timedelta(days=args.days)
                               if args.days is not None else None)
        if args.project:
            # An export records no project -- only the directory it was saved in -- so
            # the filter cannot be applied. Saying so beats silently reading everything.
            problems.append(Problem("generic", "-", "--project does not apply to an exported "
                                                    "file, which records no project; not "
                                                    "filtered"))
            project_filter = None
    else:
        since = args.since or datetime.now(timezone.utc) - timedelta(
            days=DEFAULT_DAYS if args.days is None else args.days)
        wanted = args.provider or [n for n in ADAPTERS if n != "generic"]
        for name in wanted:
            adapter = ADAPTERS.get(name)
            if adapter is None:
                problems.append(Problem(name, "-", "unknown provider"))
                continue
            for ref in adapter.discover(home, problems):
                when = datetime.fromtimestamp(ref.mtime, tz=timezone.utc)
                if when < since or (until and when > until):
                    continue
                if args.project and args.project not in (ref.project or ""):
                    continue
                refs.append(ref)

    refs.sort(key=lambda r: r.mtime, reverse=True)
    # Subagent files carry their parent's session id, so the session cap below counts
    # them as free: one session could attach any number of them. Keep the most recent
    # `MAX_SUBAGENT_FILES` per session and say how many were left out. The session's own
    # file is never counted, so a busy session is thinned rather than lost.
    attached: Counter = Counter()
    over: Counter = Counter()
    bounded: list[SessionRef] = []
    for ref in refs:
        if (ref.extra or {}).get("sidechain"):
            key = (ref.provider, ref.session_id)
            attached[key] += 1
            if attached[key] > MAX_SUBAGENT_FILES:
                over[key] += 1
                continue
        bounded.append(ref)
    for (provider, session), count in sorted(over.items()):
        problems.append(
            Problem(
                provider, session,
                f"{MAX_SUBAGENT_FILES + count} subagent files in one session; kept the "
                f"{MAX_SUBAGENT_FILES} most recent, skipped {count}",
            )
        )
    refs = bounded
    # Cap per provider, not across all of them. Pooled and truncated, a provider the
    # user is merely using *less* recently than another disappears entirely -- and
    # `discover` then reports it as "store present, no sessions in the window", which is
    # a claim about their history rather than about this run's budget.
    if args.max_sessions:
        # Counted in sessions, not files. A Claude Code subagent file carries its
        # parent's id, and counting it as a session let a few delegation-heavy sessions
        # use up the whole cap: 30 files became 21 sessions.
        kept: list[SessionRef] = []
        chosen: dict[str, set[str]] = defaultdict(set)
        skipped: dict[str, set[str]] = defaultdict(set)
        for ref in refs:
            ids = chosen[ref.provider]
            if ref.session_id in ids or len(ids) < args.max_sessions:
                ids.add(ref.session_id)
                kept.append(ref)
            else:
                skipped[ref.provider].add(ref.session_id)
        seen = Counter({p: len(v) for p, v in chosen.items()})
        dropped = Counter({p: len(v - chosen[p]) for p, v in skipped.items()})
        dropped = +dropped
        for provider, count in sorted(dropped.items()):
            problems.append(
                Problem(
                    provider, "-",
                    f"{seen[provider] + count} sessions in scope; kept the "
                    f"{args.max_sessions} most recent, skipped {count}",
                )
            )
        refs = kept
    scope = {
        "home": str(home),
        "window": {"since": iso(since), "until": iso(until)},
        "project_filter": project_filter,
    }
    if args.input:
        scope["inputs"] = list(args.input)
    return refs, problems, scope


def stream(refs: list[SessionRef], problems: list[Problem],
           scope: dict[str, Any] | None = None) -> Iterator[dict]:
    """Events from every selected session, inside the requested window.

    A file is selected by its mtime, which only says it was touched recently. A
    long-running session file touched today still holds last month's turns, and
    counting them put 36% of the events in a "last 7 days" report outside those days.
    An event with no timestamp is kept: dropping it would be a guess too.
    """
    window = (scope or {}).get("window") or {}
    since = parse_ts(window.get("since"))
    until = parse_ts(window.get("until"))
    # History replayed into a second file of the same session -- a forked rollout, a
    # subagent that inherits the parent's turns, a resumed Claude Code session -- is the
    # same turn with the same timestamp. Each adapter deduplicates within a file; only
    # here can the same turn be seen arriving from two files. A turn the person really
    # did type again carries a later timestamp and is kept.
    #
    # Tool calls and results are matched on the provider's own call id, never on content:
    # the same command run twice at the same second is two calls. Usage is matched per
    # request. Keys are stored as 16-byte digests, not the tuples they hash, because one
    # is kept per event and a tuple holding a clipped prompt is a few hundred bytes.
    seen: set[bytes] = set()
    request_tokens: dict[bytes, int] = {}
    budget = MAX_EVENTS
    read = 0
    for index, ref in enumerate(refs):
        adapter = ADAPTERS[ref.provider]
        untimed = 0
        try:
            for event in adapter.events(ref, problems):
                if read >= budget:
                    problems.append(Problem(
                        ref.provider, ref.path,
                        f"event budget of {budget} reached; stopped reading in this file, "
                        f"{len(refs) - index - 1} later file(s) not read",
                    ))
                    return
                read += 1
                when = parse_ts(event.get("ts"))
                if when and ((since and when < since) or (until and when > until)):
                    continue
                if not when and ref.provider == "generic" and event["kind"] != "session":
                    untimed += 1
                key = _replay_key(event)
                if key is not None:
                    if key in seen:
                        continue
                    seen.add(key)
                if isinstance(event.get("_requests"), dict):
                    event = _fresh_usage(event, request_tokens)
                    if event is None:
                        continue
                yield emitted(event)
        except Exception as error:  # noqa: BLE001 -- a provider change is data, not a crash
            problems.append(Problem(ref.provider, ref.path, f"adapter failed: {type(error).__name__}"))
        if untimed and (since or until):
            # An export that carries no timestamps -- every prose export -- cannot be put
            # in a window. Its events are kept, as elsewhere, and the report says so.
            problems.append(Problem(
                ref.provider, ref.path,
                f"{untimed} event(s) with no timestamp; the --since/--until window could "
                "not be applied to them",
            ))


def _digest(*parts: Any) -> bytes:
    return hashlib.blake2b(repr(parts).encode("utf-8", "replace"), digest_size=16).digest()


def _replay_key(event: dict) -> bytes | None:
    """The identity a replayed copy of `event` would share with the original, if any."""
    kind = event["kind"]
    if kind in ("tool_call", "tool_result") and event.get("_id"):
        # No session in the key: a fork replays its parent's calls under its own id.
        return _digest(event["provider"], kind, event["_id"])
    if kind in ("prompt", "assistant", "subagent", "skill") and event.get("ts"):
        return _digest(event["provider"], event["session"], kind, event["ts"],
                       event.get("text"), event.get("skill"), event.get("agent_type"))
    return None


def _fresh_usage(event: dict, seen: dict[bytes, int]) -> dict | None:
    """`event`'s usage less the requests an earlier file already counted.

    The highest count seen for a request wins, as it does within one file, so a replayed
    request adds nothing and a request that grew adds only its growth.
    """
    tokens = 0
    fresh = 0
    for request, count in event["_requests"].items():
        key = _digest(event["provider"], "request", request)
        prior = seen.get(key)
        if prior is None:
            fresh += 1
        if prior is None or count > prior:
            tokens += count - (prior or 0)
            seen[key] = count
    if not fresh and not tokens:
        return None
    return {**event, "usage": {"output_tokens": tokens, "requests": fresh}}


# --------------------------------------------------------------------------
# Subcommands.
# --------------------------------------------------------------------------


def cmd_discover(args: argparse.Namespace) -> int:
    refs, problems, scope = select(args)
    home = Path(scope["home"])
    by_provider: dict[str, dict[str, Any]] = {}
    for ref in refs:
        entry = by_provider.setdefault(
            ref.provider,
            {"provider": ref.provider, "root": ADAPTERS[ref.provider].root_hint,
             "sessions": set(), "projects": set(), "first": None, "last": None},
        )
        # Distinct session ids, not files: 1,034 "sessions" on a store of 399.
        entry["sessions"].add(ref.session_id)
        if ref.project:
            # Read from inside a file, so bounded and redacted like any emitted name.
            entry["projects"].add(_bounded(ref.project, META_CHARS))
        when = datetime.fromtimestamp(ref.mtime, tz=timezone.utc)
        entry["first"] = min(entry["first"] or when, when)
        entry["last"] = max(entry["last"] or when, when)

    found = []
    for entry in by_provider.values():
        entry["sessions"] = len(entry["sessions"])
        # These dates are when the files were last written, not when the work in them
        # happened. The summary's `busiest_days` is the one to quote as a date range.
        entry["files_modified_first"] = iso(entry.pop("first"))
        entry["files_modified_last"] = iso(entry.pop("last"))
        entry["projects"] = sorted(entry["projects"])[:50]
        found.append(entry)

    absent = []
    for name, adapter in ADAPTERS.items():
        if name == "generic" or name in by_provider:
            continue
        present = any(root.exists() for root in adapter.roots(home))
        absent.append({
            "provider": name, "root": adapter.root_hint,
            "reason": "store present, no sessions in the window" if present else "store not present",
        })
    # `--provider` restricts the run; a provider the user excluded was not examined and
    # must not be reported as having nothing.
    if args.provider:
        absent = [entry for entry in absent if entry["provider"] in set(args.provider)]

    document = {
        "schema": DISCOVERY_SCHEMA,
        "scope": scope,
        "found": sorted(found, key=lambda e: -e["sessions"]),
        "absent": absent,
        "not_parsed": [{"provider": k, "store": v} for k, v in sorted(UNSUPPORTED.items())],
        "problems": problems[:50],
        "problem_count": len(problems),
    }
    json.dump(document, sys.stdout, indent=2, sort_keys=True)
    sys.stdout.write("\n")
    return 0


def cmd_events(args: argparse.Namespace) -> int:
    refs, problems, scope = select(args)
    kinds = set(args.kind) if args.kind else set(KINDS)
    written = 0
    for event in stream(refs, problems, scope):
        if event["kind"] not in kinds:
            continue
        json.dump(event, sys.stdout, ensure_ascii=False, sort_keys=True)
        sys.stdout.write("\n")
        written += 1
        if args.limit and written >= args.limit:
            break
    for problem in problems[:50]:
        sys.stderr.write(
            f"problem: {problem['provider']}: {problem['path']}: {problem['reason']}\n"
        )
    return 0


def cmd_summary(args: argparse.Namespace) -> int:
    refs, problems, scope = select(args)

    sessions: set[tuple[str, str]] = set()
    projects: set[str] = set()
    kinds = Counter()
    tools = Counter()
    tool_sessions = defaultdict(set)
    agents = Counter()
    agent_sessions = defaultdict(set)
    skills = Counter()
    skill_sessions = defaultdict(set)
    models = Counter()
    bigrams = Counter()
    bigram_sessions = defaultdict(set)
    days = Counter()
    usage: dict[str, Counter] = defaultdict(Counter)
    # Adjacent tool pairs are the cheapest evidence that a step repeats. They
    # locate a candidate; they never establish one, because "Grep then Read"
    # 200 times is as consistent with a bounded relevance decision as it is
    # with ordinary navigation. The clustering that tells them apart is
    # semantic and is not attempted here.
    #
    # Counted as events arrive, from the last step per session. Keeping every session's
    # whole list of steps until the end held one string per tool call for the run.
    last_step: dict[tuple[str, str], str] = {}

    def step(key: tuple[str, str], name: str) -> None:
        previous = last_step.get(key)
        if previous is not None:
            bigrams[(previous, name)] += 1
            bigram_sessions[(previous, name)].add(key)
        last_step[key] = name

    for event in stream(refs, problems, scope):
        key = (event["provider"], event["session"])
        sessions.add(key)
        if event.get("project"):
            projects.add(event["project"])
        kind = event["kind"]
        kinds[kind] += 1
        if event.get("ts"):
            days[event["ts"][:10]] += 1
        if kind == "tool_call":
            tool = as_str(event.get("tool")) or "?"
            tools[tool] += 1
            tool_sessions[tool].add(key)
            step(key, tool)
        elif kind == "subagent":
            agents[event.get("agent_type") or "unspecified"] += 1
            agent_sessions[event.get("agent_type") or "unspecified"].add(key)
            step(key, "@" + (event.get("agent_type") or "unspecified"))
        elif kind == "skill":
            skill = as_str(event.get("skill")) or "?"
            skills[skill] += 1
            skill_sessions[skill].add(key)
            step(key, "/" + skill)
        elif kind == "assistant" and as_str(event.get("model")):
            # `as_str`: Gemini's `model` is not always a string, and a dict key crashed here.
            models[as_str(event.get("model"))] += 1
        elif kind == "session" and isinstance(event.get("usage"), dict):
            # Per provider, never pooled. Claude Code's transcripts record output tokens
            # per request but not a comparable input total, and Codex records both, so a
            # pooled "input vs output" put one provider's input beside another's output
            # and invited a ratio that describes nothing.
            for field, value in event["usage"].items():
                if isinstance(value, (int, float)):
                    usage[event["provider"]][field] += value

    def rank(counter: Counter, index: dict, label: str, limit: int = 25) -> list[dict]:
        return [
            {label: name if isinstance(name, str) else list(name),
             "count": count, "sessions": len(index[name])}
            for name, count in counter.most_common(limit)
        ]

    document = {
        "schema": SUMMARY_SCHEMA,
        "scope": scope,
        "totals": {
            "providers": sorted({p for p, _ in sessions}),
            "sessions": len(sessions),
            "projects": len(projects),
            "days_active": len(days),
            "events": sum(kinds.values()),
        },
        # Kept in its own object rather than flattened into `totals`: one of
        # these kinds is called "session", and flattening it silently replaced
        # the session count with the number of session-open records.
        "events_by_kind": {k: kinds[k] for k in KINDS if kinds[k]},
        "usage": {provider: dict(counts) for provider, counts in sorted(usage.items())} or None,
        "tools": rank(tools, tool_sessions, "tool"),
        "tool_pairs": rank(bigrams, bigram_sessions, "pair"),
        "subagents": rank(agents, agent_sessions, "agent_type"),
        "skills": rank(skills, skill_sessions, "skill"),
        "models": [{"model": m, "turns": c} for m, c in models.most_common(15)],
        "busiest_days": [{"day": d, "events": c} for d, c in sorted(days.items())[-30:]],
        "problems": problems[:50],
        "problem_count": len(problems),
    }
    json.dump(document, sys.stdout, indent=2, sort_keys=True)
    sys.stdout.write("\n")
    return 0


# --------------------------------------------------------------------------
# Entry point.
# --------------------------------------------------------------------------


def _moment(value: str) -> datetime:
    """An argparse type for `--since`/`--until`: a timestamp, or a usage error."""
    moment = parse_ts(value)
    if moment is None:
        raise argparse.ArgumentTypeError(f"not an ISO date or timestamp: {value!r}")
    return moment


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="transcripts.py",
        description="Discover, normalise and summarise local coding-agent transcripts. "
                    "Reads local files only; sends nothing anywhere.",
    )
    sub = parser.add_subparsers(dest="command", required=True)

    def common(p: argparse.ArgumentParser) -> None:
        p.add_argument("--provider", action="append", choices=sorted(ADAPTERS),
                       help="restrict to one provider; repeatable (default: all but generic)")
        p.add_argument("--input", action="append",
                       help="an exported transcript file to read instead of discovering "
                            "anything (.jsonl, .json, .md, .txt); repeatable")
        # No default here, so `select` can tell a window the user asked for from the one
        # it would have chosen; only the former applies to a file named with `--input`.
        p.add_argument("--days", type=int, default=None,
                       help=f"how far back to look (default: {DEFAULT_DAYS}; "
                            "not applied to --input unless given)")
        p.add_argument("--since", type=_moment, help="ISO date or timestamp; overrides --days")
        p.add_argument("--until", type=_moment, help="ISO date or timestamp")
        p.add_argument("--project", help="keep only sessions whose project path contains this")
        p.add_argument("--max-sessions", type=int, default=400,
                       help="cap on sessions read, most recent first (default: 400)")
        p.add_argument("--home", help="treat this directory as the home directory (for tests)")
        p.add_argument("--max-chars", type=int, default=DEFAULT_MAX_CHARS,
                       help=f"clip each text field (default: {DEFAULT_MAX_CHARS})")
        p.add_argument("--arg-chars", type=int, default=DEFAULT_ARG_CHARS,
                       help=f"clip each tool argument (default: {DEFAULT_ARG_CHARS})")

    p_discover = sub.add_parser("discover", help="what is present, and over what dates")
    common(p_discover)
    p_discover.set_defaults(func=cmd_discover)

    p_events = sub.add_parser("events", help="normalised events as JSONL")
    common(p_events)
    p_events.add_argument("--kind", action="append", choices=KINDS,
                          help="emit only this kind; repeatable")
    p_events.add_argument("--limit", type=int, help="stop after this many events")
    p_events.set_defaults(func=cmd_events)

    p_summary = sub.add_parser("summary", help="deterministic aggregates over the window")
    common(p_summary)
    p_summary.set_defaults(func=cmd_summary)
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    _limits["text"] = max(0, args.max_chars)
    _limits["args"] = max(0, args.arg_chars)
    try:
        return args.func(args)
    except BrokenPipeError:
        # `… | head` is normal use, not an error.
        try:
            sys.stdout.close()
        finally:
            os._exit(0)


if __name__ == "__main__":
    sys.exit(main())

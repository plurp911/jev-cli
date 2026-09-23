#!/usr/bin/env python3
"""Structurally validate the repository's own Agent Skills.

A malformed SKILL.md fails silently: the agent simply never loads the skill, and
nobody notices until a safeguard the skill was supposed to enforce is skipped. This
turns that silent failure into a build failure.

The rules come from the open Agent Skills specification, which is the portability
authority for the skills this repository ships (`docs/development/skill-authoring.md`).
A shipped skill has to work in Claude Code, Codex, Cursor, and anything else that reads
the format, so the spec wins wherever a client is more permissive than it is. Three
checks are this repository's own policy rather than the spec's, and are marked as such
below.

Checks each `.claude/skills/*/SKILL.md` and each `skills/*/SKILL.md` for:

  * YAML frontmatter delimited by `---` at the very start of the file, written in the
    subset of YAML that a strict loader reads the same way this script does (no
    duplicate keys, no plain scalar a real YAML parser would read differently or
    reject, consistent block-scalar indentation);
  * only the frontmatter keys the specification defines;
  * a `name` that is lowercase kebab-case, at most 64 characters, and equal to the
    directory name;
  * a non-empty `description` of at most 1024 characters that says both what the
    skill does and when to use it;
  * `compatibility`, when present, of 1 to 500 characters;
  * `metadata`, when present, as a map from string keys to string values;
  * `allowed-tools`, when present, as a *space*-separated list, optionally with
    `Tool(pattern)` qualifiers; a qualifier may itself contain spaces, as in
    `Bash(git status:*)`, because only whitespace outside parentheses separates;
  * a body with at least one heading, and a file of at most 500 lines;
  * for the skills under `skills/` only: every relative Markdown link, and every
    backticked `references/`, `scripts/` or `assets/` path, in SKILL.md and
    `references/*.md` resolving to a file inside that skill's own directory -- a
    Markdown link relative to the file it is in, a backticked path relative to the
    skill root.

Third-party skills installed by `scripts/skill-authoring-setup.sh` are skipped: they
are verbatim copies of pinned upstream commits, they are not ours to edit, and holding
them to our policy checks would fail this build over someone else's file. That
`--check` mode verifies them instead, against the commit they came from.

The skip applies **only** under `.claude/skills/`, which is where that script is allowed
to write. Everything under `skills/` is validated unconditionally: that tree is the
published surface, and a `PROVENANCE.md` dropped next to a `SKILL.md` would otherwise be
a one-file way to walk a skill past this gate.

Deliberately dependency-free: it runs anywhere python3 does, including a CI runner
with no pip install step.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
# Two trees, validated the same way: the skills this repository *uses* while it is being
# built, and the skills it *ships* for users of the CLI.
SKILL_DIRS = (ROOT / ".claude" / "skills", ROOT / "skills")

NAME_PATTERN = re.compile(r"^[a-z0-9]+(-[a-z0-9]+)*$")
MAX_NAME = 64
MAX_DESCRIPTION = 1024
MAX_COMPATIBILITY = 500
# The specification recommends keeping SKILL.md under 500 lines and moving the detail
# into `references/`. Enforced here rather than suggested: a control plane that grew
# into a manual stops being read, and "we will split it later" never happens.
MAX_SKILL_LINES = 500

# The complete set the specification defines. Anything else is a typo, a Claude-only
# extension, or a field another client will ignore -- each of which is worth catching
# in a skill that has to be portable.
ALLOWED_FIELDS = {
    "name",
    "description",
    "license",
    "compatibility",
    "metadata",
    "allowed-tools",
}

# `allowed-tools` is experimental and the specification defines no vocabulary for it, so
# this checks the shape rather than the names: a bare tool, optionally qualified, as in
# `Bash(git status:*)`.
TOOL_PATTERN = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*(\(.*\))?$")


def _split_tools(tools: str) -> tuple[list[str], bool, bool]:
    """Split `allowed-tools` on whitespace outside parentheses, so a qualifier such as
    `Bash(git status:*)` stays one entry. Returns (entries, balanced, top-level comma).
    """
    entries, current, depth, comma = [], "", 0, False
    for char in tools:
        if char.isspace() and depth == 0:
            if current:
                entries.append(current)
            current = ""
            continue
        if char == "," and depth == 0:
            comma = True
        depth += {"(": 1, ")": -1}.get(char, 0)
        if depth < 0:
            return entries, False, comma
        current += char
    if current:
        entries.append(current)
    return entries, depth == 0, comma


class NonString:
    """A scalar a YAML loader would not read as a string: a number, boolean, null,
    date or sequence. Kept as the raw text so a message can quote it."""

    def __init__(self, kind: str, raw: str) -> None:
        self.kind = kind
        self.raw = raw


KEY_LINE = re.compile(r"^([A-Za-z0-9_.-]+):(?:[ \t]+(.*))?$")
BLOCK_HEADER = re.compile(r"^([|>])(?:[1-9][-+]?|[-+][1-9]?)?(?:[ \t]+#.*)?$")
# A plain scalar may not start with any of these: a strict loader reads each as YAML
# syntax (flow collection, anchor, alias, tag, block scalar, directive, reserved,
# comment) and either rejects the line or silently reads something else.
PLAIN_FORBIDDEN_START = set("[]{}&*!|>%@`#")
# Plain scalars that YAML 1.1 or 1.2 loaders resolve to something other than a string.
# 1.1 matters: PyYAML and older js-yaml still read `yes`, `on` and bare dates.
NON_STRING_PLAIN = (
    ("null", re.compile(r"^(~|null|Null|NULL)$")),
    ("boolean", re.compile(r"^(true|True|TRUE|false|False|FALSE|yes|Yes|YES|no|No|NO|"
                           r"on|On|ON|off|Off|OFF|y|Y|n|N)$")),
    ("number", re.compile(r"^([-+]?[0-9][0-9_]*|0o[0-7]+|0x[0-9a-fA-F]+|"
                          r"[-+]?(\.[0-9]+|[0-9][0-9_]*(\.[0-9]*)?)([eE][-+]?[0-9]+)?|"
                          r"[-+]?\.(inf|Inf|INF)|\.(nan|NaN|NAN))$")),
    ("date", re.compile(r"^[0-9]{4}-[0-9]{1,2}-[0-9]{1,2}([Tt ].*)?$")),
)


# The escapes YAML 1.2 defines for double-quoted scalars (spec 5.7), plus an escaped
# line break. Anything else after a backslash -- `\q`, `\x4`, `\u12` -- is an error
# to a strict loader, however forgiving a lenient one is about it.
DQ_ESCAPE = re.compile(
    r"\\(?:[0abtnvfre \t\"/\\N_LP\n]|x[0-9A-Fa-f]{2}|u[0-9A-Fa-f]{4}|U[0-9A-Fa-f]{8})"
)
_DQ_SIMPLE = {
    "0": "\0", "a": "\a", "b": "\b", "t": "\t", "\t": "\t", "n": "\n", "v": "\v",
    "f": "\f", "r": "\r", "e": "\x1b", " ": " ", '"': '"', "/": "/", "\\": "\\",
    "N": "\x85", "_": "\xa0", "L": "\u2028", "P": "\u2029", "\n": "",
}


def _unescape(match: re.Match[str]) -> str:
    escape = match.group()[1:]
    if escape[0] in "xuU":
        return chr(int(escape[1:], 16))
    return _DQ_SIMPLE[escape]


def _indent(line: str) -> int:
    return len(line) - len(line.lstrip(" "))


def _read_value(
    label: str, value: str, conts: list[tuple[int, str]], parent: int, problems: list[str]
) -> object:
    """Read one value: the text after `key:` plus the more-indented lines under it.

    Returns a string, a mapping (only `metadata` may legitimately be one), a NonString,
    or None for YAML null (a key with no value). `parent` is the key's own
    indentation; every line in `conts` is deeper than it.
    """
    body = [(n, line) for n, line in conts if line.strip()]

    if BLOCK_HEADER.match(value):
        if not body:
            return ""
        digit = re.search(r"[1-9]", value.split("#")[0])
        want = parent + int(digit.group()) if digit else _indent(body[0][1])
        for n, line in body:
            if _indent(line) < want:
                problems.append(
                    f"frontmatter line {n}: bad indentation under block scalar `{label}` "
                    f"(expected at least {want} spaces, got {_indent(line)}); a strict "
                    "YAML loader rejects this"
                )
                break
        return " ".join(line.strip() for _, line in body)

    if not value and not body:
        return None

    if value == "":
        first = body[0][1].strip()
        if first == "-" or first.startswith("- "):
            return NonString("sequence", first)
        if KEY_LINE.match(first):
            return _read_mapping(label, body, problems)
        value, body = first, body[1:]

    if value[0] in "\"'":
        quote = value[0]
        # Joined on newlines so an escaped line break (`\` at the end of a line) stays
        # visible to the escape check; every other line break folds to a space below.
        text = "\n".join([value] + [line.strip() for _, line in body])
        inner = r"((?:[^']|'')*)" if quote == "'" else r'((?:[^"\\]|\\.)*)'
        match = re.match(rf"^{quote}{inner}{quote}(?:[ \t]+#.*)?$", text, re.DOTALL)
        if not match:
            problems.append(
                f"`{label}` has an unterminated or malformed {quote}-quoted value, or "
                "text after the closing quote"
            )
            return text.replace("\n", " ")
        inner = match.group(1)
        if quote == "'":
            return inner.replace("\n", " ").replace("''", "'")
        for bad in sorted({m.group() for m in re.finditer(r"\\.", inner, re.DOTALL)
                           if not DQ_ESCAPE.match(inner, m.start())}):
            problems.append(
                f"`{label}` has the escape `{bad}` in a double-quoted value, which YAML "
                "1.2 does not define and a strict loader rejects; write a literal "
                "backslash as `\\\\`, or use single quotes or a `>-` block scalar"
            )
        return DQ_ESCAPE.sub(_unescape, inner).replace("\n", " ")

    if value[0] in PLAIN_FORBIDDEN_START or re.match(r"^[-?:]([ \t]|$)", value):
        problems.append(
            f"`{label}` starts with {value[0]!r}, which YAML reads as syntax rather than "
            "text; quote the value or use a `>-` block scalar"
        )
        return value

    pieces = [value] + [line.strip() for _, line in body]
    for piece in pieces:
        if ": " in piece or piece.endswith(":"):
            problems.append(
                f"`{label}` is a plain scalar containing ': ', which a strict YAML "
                "loader rejects as a nested mapping; quote the value or use a `>-` "
                "block scalar"
            )
            break
        if " #" in piece or "\t#" in piece:
            problems.append(
                f"`{label}` contains ' #', which YAML reads as the start of a comment "
                "and silently drops the rest; quote the value or use a `>-` block scalar"
            )
            break
    text = " ".join(pieces)
    for kind, pattern in NON_STRING_PLAIN:
        if pattern.match(text):
            return NonString(kind, text)
    return text


def _read_mapping(label: str, lines: list[tuple[int, str]], problems: list[str]) -> dict:
    """Read an indented block mapping (in practice, `metadata`)."""
    mapping: dict[str, object] = {}
    indent = _indent(lines[0][1])
    entries: list[tuple[int, str, str, list[tuple[int, str]]]] = []
    for n, line in lines:
        if _indent(line) > indent and entries:
            entries[-1][3].append((n, line))
            continue
        match = KEY_LINE.match(line.strip())
        if _indent(line) < indent or not match:
            problems.append(
                f"frontmatter line {n}: `{line.strip()}` under `{label}` is not a "
                "`key: value` entry at the mapping's indentation"
            )
            continue
        entries.append((n, match.group(1), (match.group(2) or "").rstrip(), []))
    for n, key, value, conts in entries:
        if key in mapping:
            problems.append(f"frontmatter line {n}: duplicate key `{label}.{key}`")
        mapping[key] = _read_value(f"{label}.{key}", value, conts, indent, problems)
    return mapping


def parse_frontmatter(text: str) -> tuple[dict[str, object], str, list[str]] | None:
    """Return (fields, body, problems), or None when the frontmatter is missing or
    unterminated.

    This is a minimal reader for the `key: value` frontmatter that SKILL.md uses,
    including YAML block scalars (`>` and `|`) and one level of block mapping for
    `metadata`. It is not a general YAML parser and does not try to be. What it does
    do is refuse the constructs where it and a strict loader -- the one another agent
    client uses -- would disagree, so that "valid here" means "loads everywhere".
    """
    if not text.startswith("---\n"):
        return None
    end = text.find("\n---\n", 4)
    if end == -1:
        return None

    problems: list[str] = []
    groups: list[tuple[int, str, str, list[tuple[int, str]]]] = []
    # Line 1 is the opening `---`.
    for n, line in enumerate(text[4:end].split("\n"), start=2):
        if "\t" in line[: len(line) - len(line.lstrip())]:
            problems.append(f"frontmatter line {n}: tab indentation, which YAML forbids")
            continue
        if not line.strip() or line.startswith("#"):
            if groups:
                groups[-1][3].append((n, line))
            continue
        if line[0] == " ":
            if not groups:
                problems.append(f"frontmatter line {n}: indented line before any key")
            else:
                groups[-1][3].append((n, line))
            continue
        match = KEY_LINE.match(line.rstrip())
        if not match:
            problems.append(
                f"frontmatter line {n}: `{line}` is not a `key: value` pair (a key, a "
                "colon, then a space or the end of the line)"
            )
            continue
        groups.append((n, match.group(1), (match.group(2) or "").rstrip(), []))

    fields: dict[str, object] = {}
    for n, key, value, conts in groups:
        if key in fields:
            problems.append(
                f"frontmatter line {n}: duplicate key `{key}`; strict YAML loaders reject "
                "the file, lenient ones silently keep one of the values"
            )
        # A comment line at column 0 ends nothing in a block scalar only if it is
        # less indented, which a column-0 line always is: drop it from the value.
        conts = [(i, line) for i, line in conts if not line.startswith("#")]
        fields[key] = _read_value(key, value, conts, 0, problems)
    return fields, text[end + 5 :], problems


def _describe(value: object) -> str:
    if isinstance(value, NonString) and value.kind == "sequence":
        return f"a list ({value.raw!r} ...)"
    if isinstance(value, NonString):
        return f"a {value.kind} ({value.raw!r}); quote it to make it a string"
    if isinstance(value, dict):
        return "a mapping"
    return "empty (YAML null)"


def _string(fields: dict[str, object], key: str, problems: list[str]) -> str:
    """The field as a string, or "" when absent or null; reports any other type."""
    value = fields.get(key)
    if value is None or isinstance(value, str):
        return value or ""
    problems.append(f"`{key}` must be a string, but YAML reads it as {_describe(value)}")
    return ""


def check(path: Path) -> list[str]:
    problems: list[str] = []
    text = path.read_text(encoding="utf-8")

    lines = text.count("\n") + 1
    if lines > MAX_SKILL_LINES:
        problems.append(
            f"SKILL.md is {lines} lines, over the {MAX_SKILL_LINES} limit -- "
            "move detail into references/ and leave a pointer"
        )

    parsed = parse_frontmatter(text)
    if parsed is None:
        return ["missing or unterminated `---` YAML frontmatter at the start of the file"]
    fields, body, problems_in_yaml = parsed
    problems.extend(problems_in_yaml)

    for field in sorted(set(fields) - ALLOWED_FIELDS):
        problems.append(
            f"frontmatter key {field!r} is not in the Agent Skills specification "
            f"({', '.join(sorted(ALLOWED_FIELDS))})"
        )

    name = _string(fields, "name", problems)
    if not name:
        problems.append("frontmatter is missing `name`")
    else:
        if not NAME_PATTERN.match(name):
            problems.append(
                f"`name` must be lowercase alphanumerics and single hyphens, got {name!r}"
            )
        if len(name) > MAX_NAME:
            problems.append(f"`name` is {len(name)} characters, over the {MAX_NAME} limit")
        if name != path.parent.name:
            problems.append(f"`name` {name!r} does not match directory {path.parent.name!r}")

    description = _string(fields, "description", problems)
    if not description:
        problems.append("frontmatter is missing `description`")
    else:
        if len(description) > MAX_DESCRIPTION:
            problems.append(
                f"`description` is {len(description)} characters, over the {MAX_DESCRIPTION} limit"
            )
        # Repository policy, not a specification rule: a description that does not say
        # *when* to reach for the skill is a description the agent cannot route on, and
        # routing is the whole job of this field.
        if "use " not in description.lower():
            problems.append(
                "`description` should say when to use the skill, not only what it does"
            )

    _string(fields, "license", problems)

    # The specification: "Must be 1-500 characters if provided". An empty value is not
    # "absent"; it is a present field that breaks the lower bound.
    if "compatibility" in fields:
        compatibility = fields["compatibility"]
        if isinstance(compatibility, (NonString, dict)):
            problems.append(
                "`compatibility` must be a string, but YAML reads it as "
                f"{_describe(compatibility)}"
            )
        elif not compatibility:
            problems.append(
                "`compatibility` is empty; the specification requires 1 to 500 "
                "characters when it is present -- fill it in or remove the key"
            )
        elif len(compatibility) > MAX_COMPATIBILITY:
            problems.append(
                f"`compatibility` is {len(compatibility)} characters, "
                f"over the {MAX_COMPATIBILITY} limit"
            )

    # The specification: "A map from string keys to string values". Nested mappings,
    # lists, and unquoted numbers or booleans all load as something else.
    if "metadata" in fields:
        metadata = fields["metadata"]
        if not isinstance(metadata, dict):
            read_as = "a string" if isinstance(metadata, str) else _describe(metadata)
            problems.append(
                "`metadata` must be a map from string keys to string values, but YAML "
                f"reads it as {read_as}"
            )
        else:
            for key, value in metadata.items():
                if not isinstance(value, str):
                    problems.append(
                        f"`metadata.{key}` must be a string value, but YAML reads it as "
                        f"{_describe(value)}"
                    )

    tools = _string(fields, "allowed-tools", problems)
    if tools:
        # The specification says space-separated. Claude Code also accepts commas, which
        # is exactly why this is checked: a comma-separated list is silently
        # Claude-specific, and a shipped skill has to load everywhere. A comma inside a
        # qualifier is part of its pattern, not a separator.
        entries, balanced, comma = _split_tools(tools)
        if not balanced:
            problems.append(f"`allowed-tools` has unbalanced parentheses: {tools!r}")
        elif comma:
            problems.append(
                "`allowed-tools` must be space-separated per the Agent Skills "
                f"specification, got a comma-separated list: {tools!r}"
            )
        else:
            for tool in entries:
                if not TOOL_PATTERN.match(tool):
                    problems.append(f"`allowed-tools` entry {tool!r} is not a tool name")

    # Repository policy, not a specification rule: the body is free-form per the spec,
    # but a skill with no heading has no structure for an agent to navigate.
    if not re.search(r"^#{1,6} ", body, re.MULTILINE):
        problems.append("body has no Markdown heading")

    return problems


FENCE = re.compile(r"^ {0,3}(```|~~~).*?^ {0,3}\1[^\n]*$", re.MULTILINE | re.DOTALL)
# `[text](target)` and `![alt](target)`, with link text allowed to wrap across lines and
# an optional `"title"` after the target.
MD_LINK = re.compile(r"!?\[[^\]]*\]\(\s*<?([^)\s>]+)>?(?:\s+\"[^\"]*\")?\s*\)")
# A backticked path into one of the spec's three conventional directories. Only the
# first whitespace-separated token counts, so `scripts/x.py discover` checks the script.
TICK_PATH = re.compile(r"`((?:references|scripts|assets)/[^`\s]*)[^`]*`")
URL_SCHEME = re.compile(r"^[A-Za-z][A-Za-z0-9+.-]*:")


def check_references(skill: Path) -> list[str]:
    """Every file a shipped skill points at must ship with it.

    A skill under `skills/` is installed on its own, into a directory whose parent is
    not this repository, so a target that is missing, or that `../` walks out of the
    skill to reach, is a dead end for the agent reading it.

    Two rules, one per kind of reference. A Markdown link resolves against the file it
    is in and nothing else, because that is how every renderer and every agent that
    follows the link resolves it: `[x](references/foo.md)` inside `references/a.md`
    means `references/references/foo.md`, and is broken. A backticked path is prose,
    not a link, and follows the specification's convention that file references are
    "relative paths from the skill root", so `scripts/x.py` means the same file from
    SKILL.md and from `references/a.md`.
    """
    problems: list[str] = []
    root = skill.resolve()
    documents = [skill / "SKILL.md", *sorted((skill / "references").glob("*.md"))]
    for document in documents:
        # Blank out fenced code, keeping line numbers: a path in an example is not a
        # reference, and fence backticks would confuse the inline-code match.
        text = document.read_text(encoding="utf-8")
        text = FENCE.sub(lambda m: "\n" * m.group().count("\n"), text)
        rel = document.relative_to(skill)
        found = [(m, document.parent) for m in MD_LINK.finditer(text)]
        found += [(m, skill) for m in TICK_PATH.finditer(text)]
        for match, base in found:
            target = match.group(1)
            if target.startswith("#") or URL_SCHEME.match(target):
                continue
            if any(c in target for c in "*?<>{}$"):
                continue  # a glob or a placeholder, not a file
            target = target.split("#", 1)[0]
            line = text.count("\n", 0, match.start()) + 1
            resolved = (base / target).resolve()
            if resolved != root and root not in resolved.parents:
                problems.append(
                    f"{rel}:{line}: `{target}` points outside the skill directory; the "
                    "skill is installed on its own, so the target will not be there"
                )
            elif not resolved.exists():
                where = "the skill root" if base == skill else f"{rel.parent}/"
                problems.append(
                    f"{rel}:{line}: `{target}` does not exist (resolved relative to "
                    f"{where})"
                )
    return problems


def main() -> int:
    skills: list[Path] = []
    skipped: list[Path] = []
    misnamed: list[Path] = []

    for directory in SKILL_DIRS:
        # Only the development tree may hold a third-party skill. `skills/` is what users
        # install, so nothing there is exempt from anything and the marker is not even
        # consulted -- otherwise adding one file would be enough to smuggle an unchecked
        # skill past the gate that exists to stop exactly that.
        may_be_third_party = directory == ROOT / ".claude" / "skills"
        for candidate in sorted(directory.glob("*/")):
            # A third-party skill carries the provenance file that
            # `scripts/skill-authoring-setup.sh` writes. It is not ours to hold to our
            # policy; `--check` verifies it against its pinned commit instead.
            if may_be_third_party and (candidate / "PROVENANCE.md").exists():
                skipped.append(candidate)
                continue
            if (candidate / "SKILL.md").exists():
                skills.append(candidate / "SKILL.md")
            elif (candidate / "skill.md").exists():
                # Lowercase loads in some clients and not others. Catching it here beats
                # discovering it as a skill that mysteriously never triggers.
                misnamed.append(candidate / "skill.md")

    if not skills:
        listed = ", ".join(str(d) for d in SKILL_DIRS)
        print(f"no skills found under {listed}", file=sys.stderr)
        return 1

    failed = 0
    for path in misnamed:
        failed += 1
        rel = path.relative_to(ROOT)
        print(f"{rel}: must be named SKILL.md, in capitals", file=sys.stderr)

    for path in skills:
        problems = check(path)
        # Only the shipped tree: the development skills under `.claude/skills/` run
        # inside this checkout and legitimately name repository paths such as
        # `scripts/verify.sh`.
        if path.parent.parent == ROOT / "skills":
            problems += check_references(path.parent)
        rel = path.relative_to(ROOT)
        if problems:
            failed += 1
            for problem in problems:
                print(f"{rel}: {problem}", file=sys.stderr)
        else:
            print(f"ok  {rel}")

    for path in skipped:
        print(f"skip {path.relative_to(ROOT)} (third-party, pinned; see PROVENANCE.md)")

    if failed:
        print(f"\n{failed} skill(s) failed validation", file=sys.stderr)
        return 1
    print(f"\n{len(skills)} skill(s) valid, {len(skipped)} third-party skill(s) skipped")
    return 0


if __name__ == "__main__":
    sys.exit(main())

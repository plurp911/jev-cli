#!/usr/bin/env python3
"""Check that every `--require` in the cookbook can actually be evaluated.

The mistake this exists to catch is asking a Noul for a `confidence`. The API returns
none for a Noul, so the gate is `unevaluable` for every row -- which, in `jev map`, means
every record is diverted to the review file, the main stream is empty, and the run still
exits 0. It is the single easiest mistake to make with this feature, it is what
`docs/commands.md` and ADR-0010 warn about, and it shipped in the example that
demonstrates the feature. Prose warning people off a mistake does not stop the mistake
being committed; this does.

For every fenced command in `examples/README.md` that names both a request file and a
`--require` expression, each `<id>.<field>` path in the expression is checked against
the question's actual type.
"""

from __future__ import annotations

import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
README = ROOT / "examples" / "README.md"

# What each primitive actually answers with. From the official documentation:
# https://docs.typesafe.ai/primitives and https://docs.typesafe.ai/confidence --
# `confidence` exists on Choice and Score only, because it summarizes the spread of a
# distribution over several outcomes. A Noul's single probability describes its whole
# two-outcome distribution, so there is nothing to summarize and the API returns none.
FIELDS = {
    "noul": {"noul"},
    "choice": {"choice", "confidence", "probabilities"},
    "score": {"score", "confidence", "probabilities"},
}

# A shell command may be split across lines with a trailing backslash.
COMMAND = re.compile(r"```(?:sh|console|yaml)\n(.*?)```", re.S)
REQUEST = re.compile(r"-r\s+(examples/requests/[\w.-]+\.json)")
# Both quoting styles, and an unquoted single-token expression. Matching only `'...'`
# meant rewriting the cookbook's gate with double quotes made this check silently stop
# looking at it -- the exact failure mode it exists to prevent.
REQUIRE = re.compile(r"--require\s+(?:'([^']*)'|\"([^\"]*)\"|(\S+))")
PATH = re.compile(r"\b([A-Za-z_][\w-]*)\.([A-Za-z_][\w-]*)")

# Words that are grammar, not question ids.
KEYWORDS = {"and", "or", "not"}


def main() -> int:
    problems: list[str] = []
    checked = 0

    text = README.read_text()
    for block in COMMAND.findall(text):
        joined = block.replace("\\\n", " ")
        for command in joined.split("\n"):
            request = REQUEST.search(command)
            require = REQUIRE.search(command)
            if not (request and require):
                # A `--require` with no request file on the same line cannot be checked,
                # and silently skipping it is how a gate escapes this check. Say so.
                if REQUIRE.search(command) and "--require" in command:
                    problems.append(
                        f"`--require` in `{command.strip()[:60]}…` names no request "
                        "file on the same command, so its paths cannot be checked"
                    )
                continue
            expression = next(g for g in require.groups() if g is not None)
            path = ROOT / request.group(1)
            if not path.exists():
                problems.append(f"{request.group(1)} does not exist")
                continue
            document = json.loads(path.read_text())
            questions = document.get("questions", document)
            types = {
                name: body.get("type")
                for name, body in questions.items()
                if isinstance(body, dict)
            }

            for identifier, field in PATH.findall(expression):
                if identifier in KEYWORDS:
                    continue
                checked += 1
                kind = types.get(identifier)
                if kind is None:
                    problems.append(
                        f"{request.group(1)}: `--require` names `{identifier}`, "
                        f"which is not a question in it (it has: "
                        f"{', '.join(sorted(types))})"
                    )
                elif field not in FIELDS.get(kind, set()):
                    allowed = ", ".join(sorted(FIELDS.get(kind, set())))
                    extra = ""
                    if kind == "noul" and field == "confidence":
                        extra = (
                            ". A Noul has no confidence: this gate is `unevaluable` for "
                            "every row. Write a band on the probability instead, e.g. "
                            f"`{identifier}.noul > 0.7 or {identifier}.noul < 0.3`"
                        )
                    problems.append(
                        f"{request.group(1)}: `{identifier}` is a {kind}, so "
                        f"`{identifier}.{field}` cannot be evaluated; a {kind} answers "
                        f"with {allowed}{extra}"
                    )

    if problems:
        print("the cookbook contains a gate that cannot be evaluated:")
        for problem in problems:
            print(f"  - {problem}")
        return 1
    if checked == 0:
        print("FAIL  found no --require expression to check; the parser probably broke")
        return 1
    print(f"ok    {checked} gate path(s) in the cookbook are addressable")
    return 0


if __name__ == "__main__":
    sys.exit(main())

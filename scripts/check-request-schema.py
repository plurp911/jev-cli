#!/usr/bin/env python3
"""Check request-file examples and the advertised MCP media schema contracts.

The request-file format is the official TypeSafe API request body, not an invention of
this CLI, so it carries no `version` field: adding one would make the file invalid as an
API body. The schema is versioned by its `$id` instead. That makes it a second,
independent statement of the same format -- and a second statement is only useful while
something keeps it true. This is that something.

Every file in `examples/requests/` must validate. The negative cases below must not:
a schema that accepts everything would pass the first check and tell nobody anything.
The MCP checks also validate per-call and map record-level video metadata, with
positive source FPS/duration and processor sampling FPS, against the five advertised
input schemas. Valid controls must pass before zero-value counterexamples are tested.

Without `jsonschema`, exit 77 reports that validation did not run. The repository
verifier records a skip locally and refuses a push until the module is installed.
"""

from __future__ import annotations

import json
import copy
import base64
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
SCHEMA = ROOT / "schema" / "request.schema.json"
EXAMPLES = ROOT / "examples" / "requests"

# Documents the schema must reject, with the reason it must reject them. Each is a
# mistake `crates/jev-cli/src/request.rs` already refuses, so the two agree.
REJECTED: list[tuple[str, dict]] = [
    ("an unknown top-level field", {"questions": {"a": {"type": "noul", "instructions": "?"}}, "extra": 1}),
    ("an unknown question type", {"questions": {"a": {"type": "rank", "instructions": "?"}}}),
    ("a question with no instructions", {"questions": {"a": {"type": "noul"}}}),
    ("an unknown question field", {"questions": {"a": {"type": "noul", "instructions": "?", "weight": 1}}}),
    ("a choice with no criteria", {"questions": {"a": {"type": "choice", "instructions": "?"}}}),
    ("a choice with one option", {"questions": {"a": {"type": "choice", "instructions": "?", "criteria": {"x": None}}}}),
    ("a score with one level", {"questions": {"a": {"type": "score", "instructions": "?", "criteria": ["only"]}}}),
    ("a score with eleven levels", {"questions": {"a": {"type": "score", "instructions": "?", "criteria": [str(n) for n in range(11)]}}}),
    ("no questions at all", {"questions": {}}),
    ("numeric instructions", {"questions": {"a": {"type": "noul", "instructions": 3}}}),
    ("noul criteria with a side that is not true or false", {"questions": {"a": {"type": "noul", "instructions": "?", "criteria": {"maybe": "x"}}}}),
    # The direction that actually bites an editor user: a file that validates green in an
    # IDE and is then refused at run time. `minLength` does not trim, and `jev` does.
    ("whitespace-only instructions", {"questions": {"a": {"type": "noul", "instructions": "   "}}}),
    ("a whitespace-only score level", {"questions": {"a": {"type": "score", "instructions": "?", "criteria": ["   ", "b"]}}}),
    ("a whitespace-only choice description", {"questions": {"a": {"type": "choice", "instructions": "?", "criteria": {"x": "  ", "y": None}}}}),
    ("a whitespace-only noul side", {"questions": {"a": {"type": "noul", "instructions": "?", "criteria": {"true": "  "}}}}),
    ("a blank question id", {"questions": {"  ": {"type": "noul", "instructions": "?"}}}),
    ("a question id with a control character", {"questions": {"a\u0007b": {"type": "noul", "instructions": "?"}}}),
    ("a blank choice option name", {"questions": {"a": {"type": "choice", "instructions": "?", "criteria": {"  ": None, "b": None}}}}),
    ("a choice option name with a control character", {"questions": {"a": {"type": "choice", "instructions": "?", "criteria": {"a\u0007b": None, "b": None}}}}),
    ("a question id with a C1 control character", {"questions": {"a\u0080b": {"type": "noul", "instructions": "?"}}}),
    # Upper boundaries. The check tested that 255 options and 10 levels are accepted but
    # never that 256 and 11 are refused, so deleting `maxProperties`/`maxItems` from the
    # schema left it passing.
    ("a choice at 256 options", {"questions": {"a": {"type": "choice", "instructions": "?", "criteria": {f"o{n}": None for n in range(256)}}}}),
    ("a score at 11 levels", {"questions": {"a": {"type": "score", "instructions": "?", "criteria": [str(n) for n in range(11)]}}}),
    ("a choice at 1 option", {"questions": {"a": {"type": "choice", "instructions": "?", "criteria": {"only": None}}}}),
    ("a model with a control character", {"model": "a\u0007b", "questions": {"a": {"type": "noul", "instructions": "?"}}}),
    ("a blank model", {"model": "   ", "questions": {"a": {"type": "noul", "instructions": "?"}}}),
    ("noul criteria describing neither side", {"questions": {"a": {"type": "noul", "instructions": "?", "criteria": {}}}}),
    ("noul criteria with both sides null", {"questions": {"a": {"type": "noul", "instructions": "?", "criteria": {"true": None, "false": None}}}}),
]

# Documents the schema must ACCEPT. A schema that rejects a valid file is worse than a
# lax one: it makes an editor red-underline something `jev` runs happily.
ACCEPTED: list[tuple[str, dict]] = [
    ("a full document with a question named `type`", {"questions": {"type": {"type": "noul", "instructions": "?"}}}),
    ("a bare questions map with a question named `questions`", {"questions": {"type": "noul", "instructions": "?"}}),
    ("a question named `state`", {"questions": {"state": {"type": "noul", "instructions": "?"}}}),
    ("noul criteria describing only the false side", {"questions": {"a": {"type": "noul", "instructions": "?", "criteria": {"false": "x"}}}}),
    ("null noul criteria", {"questions": {"a": {"type": "noul", "instructions": "?", "criteria": None}}}),
    ("structured instructions", {"questions": {"a": {"type": "noul", "instructions": {"task": "?", "note": "x"}}}}),
    ("a choice at 255 options", {"questions": {"a": {"type": "choice", "instructions": "?", "criteria": {f"o{n}": None for n in range(255)}}}}),
    ("a score at 10 levels", {"questions": {"a": {"type": "score", "instructions": "?", "criteria": [str(n) for n in range(10)]}}}),
    # `jev` trims before measuring, so this is a valid 3-character model name. A plain
    # `maxLength` in the schema measured it before trimming and refused it.
    ("a model padded past 128 characters that trims to a valid one", {"model": " " * 129 + "jev", "questions": {"a": {"type": "noul", "instructions": "?"}}}),
]


def check_mcp_media_contract(jsonschema) -> int:
    """Check the advertised timing contract, including map's record-level media."""
    encoded = base64.b64encode(
        (ROOT / "crates/jev-core/tests/fixtures/two-by-three.png").read_bytes()
    ).decode("ascii")
    frame = {"content_type": "image/png", "base64": encoded}
    video = {"frames": [frame, frame], "metadata": {
        "fps": 30, "total_num_frames": 90, "frames_indices": [0, 60], "duration": 3}}
    question = {"id": "q", "type": "noul", "instructions": "Motion?"}
    cases = {
        "noul": {"state": "clip", "instructions": "Motion?"},
        "choice": {"state": "clip", "instructions": "Direction?", "options": [
            {"name": "left"}, {"name": "right"}]},
        "score": {"state": "clip", "instructions": "Motion?", "levels": ["still", "moving"]},
        "ask": {"state": "clip", "questions": [question]},
        "map": {"questions": [question], "records": [{"state": "clip"}]},
    }
    failures = 0
    for tool, base in cases.items():
        schema = json.loads((ROOT / "crates/jev-cli/src/mcp/schema" / (tool + ".input.json")).read_text())
        validator_type = jsonschema.validators.validator_for(schema)
        validator_type.check_schema(schema)
        validator = validator_type(schema)
        for record_level in ([False, True] if tool == "map" else [False]):
            arguments = copy.deepcopy(base)
            media = arguments["records"][0] if record_level else arguments
            media["videos"] = [copy.deepcopy(video)]
            location = "record" if record_level else "template"
            if not validator.is_valid(arguments):
                failures += 1
                print(f"FAIL  MCP {tool} schema rejects valid {location} video timing")
            for field in ("fps", "duration"):
                invalid = copy.deepcopy(arguments)
                target = invalid["records"][0] if record_level else invalid
                target["videos"][0]["metadata"][field] = 0
                if validator.is_valid(invalid):
                    failures += 1
                    print(f"FAIL  MCP {tool} schema accepts zero {location} video {field}")
            invalid = copy.deepcopy(arguments)
            invalid["media_kwargs"] = {"fps": 0}
            if validator.is_valid(invalid):
                failures += 1
                print(f"FAIL  MCP {tool} schema accepts zero sampling fps")
    if not failures:
        print("ok    all five MCP schemas accept video timing and reject zero cadence/duration")
    return failures


def main() -> int:
    try:
        import jsonschema
    except ImportError:
        print("skip  jsonschema is not installed; cannot validate the request schema")
        return 77

    schema = json.loads(SCHEMA.read_text())
    validator_for = jsonschema.validators.validator_for(schema)
    validator_for.check_schema(schema)
    validator = validator_for(schema)

    failures = check_mcp_media_contract(jsonschema)

    examples = sorted(EXAMPLES.glob("*.json"))
    if not examples:
        print(f"FAIL  no examples in {EXAMPLES}")
        return 1
    for path in examples:
        errors = list(validator.iter_errors(json.loads(path.read_text())))
        if errors:
            failures += 1
            print(f"FAIL  {path.relative_to(ROOT)} does not match the schema:")
            for error in errors[:3]:
                print(f"        {error.message}")

    for reason, document in REJECTED:
        if validator.is_valid(document):
            failures += 1
            print(f"FAIL  the schema accepts {reason}, which `jev` refuses")

    for reason, document in ACCEPTED:
        errors = list(validator.iter_errors(document))
        if errors:
            failures += 1
            print(f"FAIL  the schema rejects {reason}, which `jev` accepts:")
            for error in errors[:2]:
                print(f"        {error.message}")

    if failures:
        return 1
    print(
        f"ok    {len(examples)} example(s) match the schema, "
        f"{len(REJECTED)} malformed document(s) are rejected, "
        f"and {len(ACCEPTED)} valid document(s) are accepted"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())

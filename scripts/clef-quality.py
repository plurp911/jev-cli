#!/usr/bin/env python3
"""Explicit, bounded Clef benchmark on 32 original labelled synthetic examples.

This measures only the supplied shape family, never production accuracy. Plan is
offline; run sends the generated examples to the explicitly selected provider.
"""
from __future__ import annotations

import argparse
import base64
import importlib.util
import json
import math
from pathlib import Path
import struct
import sys
import tempfile
import zlib

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("clef_smoke", Path(__file__).with_name("clef-live.py"))
smoke = importlib.util.module_from_spec(spec)
spec.loader.exec_module(smoke)
QUESTIONS = {"visible": {"type": "noul", "instructions": "Is a red square present?"}}
ROW_COUNT = 32


def shape_png(shape, color, left, size):
    """Render one original shape with no external assets or image dependencies."""
    pixels = bytearray()
    for y in range(256):
        pixels.append(0)
        for x in range(256):
            filled = (left <= x < left + size and 80 <= y < 80 + size) if shape == "square" else (
                (x - (left + size / 2)) ** 2 + (y - (80 + size / 2)) ** 2 < (size / 2) ** 2)
            pixels.extend((230, 20, 20) if filled and color == "red" else
                          (20, 20, 230) if filled else (255, 255, 255))
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 256, 256, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(bytes(pixels), 9)) + chunk(b"IEND", b""))


def rows(provider):
    result = []
    for shape in ("square", "circle"):
        for color in ("red", "blue"):
            for left in (16, 56, 96, 136):
                for size in (40, 72):
                    row = {"schema": "jev.eval.row/v1", "id": f"{shape}-{color}-{left}-{size}",
                           "state": "Inspect only the supplied shape.",
                           "labels": {"visible": shape == "square" and color == "red"}}
                    if provider == "llamacpp":
                        row["state"] = f"The only shape is a {color} {shape}. Its side or diameter is {size}; left position is {left}."
                    else:
                        row["images"] = [{"content_type": "image/png", "base64":
                                          base64.b64encode(shape_png(shape, color, left, size)).decode("ascii")}]
                    result.append(row)
    return result


def measure(document, expected):
    smoke.require(type(document) is dict and document.get("schema") == "jev.eval/v1", "quality evaluation schema differs")
    summary = document.get("rows", {})
    smoke.require(type(summary) is dict and type(summary.get("evaluated")) is int
                  and type(summary.get("failed")) is int
                  and summary.get("evaluated") == ROW_COUNT and summary.get("failed") == 0
                  and summary.get("stopped_early") is False and summary.get("interrupted") is False,
                  "quality evaluation did not finish every row")
    questions = document.get("questions", {})
    smoke.require(type(questions) is dict and set(questions) == {"visible"}, "quality question set differs")
    section = questions["visible"]
    smoke.require(type(section) is dict and section.get("type") == "noul", "quality answer type differs")
    observations = section.get("rows", [])
    smoke.require(type(section.get("n")) is int and section.get("n") == ROW_COUNT
                  and type(observations) is list and len(observations) == ROW_COUNT,
                  "quality observations are incomplete")
    brier, loss, agreed = 0.0, 0.0, 0
    for observation, row in zip(observations, expected):
        smoke.require(type(observation) is dict, "quality observation differs")
        label = row["labels"]["visible"]
        smoke.require(observation.get("id") == row["id"] and observation.get("label") is label,
                      "quality labels or order differ")
        predicted = observation.get("predicted")
        smoke.probability(predicted)
        brier += (predicted - int(label)) ** 2
        loss -= math.log(max(predicted if label else 1 - predicted, 1e-15))
        agreed += (predicted >= 0.5) == label
    input_kind = "text" if all("images" not in row for row in expected) else "image"
    return {"rows": ROW_COUNT, "positives": 8, "negatives": 24,
            "input_kind": input_kind,
            "evaluation_task": "literal_text_reading" if input_kind == "text" else "synthetic_shape_vision",
            "comparable_across_input_kinds": False,
            "agreement_at_0_5": agreed / ROW_COUNT, "brier_score": brier / ROW_COUNT,
            "log_loss_clipped_at_1e_15": loss / ROW_COUNT, "generalizable": False,
            "limitation": (
                "Text-only sanity check: the input states the color and shape; these metrics do not measure vision and cannot be compared with image runs."
                if input_kind == "text" else
                "Only this original synthetic image shape family; no production calibration claim or comparison with literal text runs")}


def run(config, runner=smoke.bounded_run):
    smoke.configuration(config)
    smoke.require(config["max_requests"] >= ROW_COUNT, "quality run exceeds max-requests; nothing was sent")
    config = dict(config)
    if config.get("key_file"):
        config["key_file"] = Path(config["key_file"]).resolve()
    expected = rows(config["provider"])
    result = {"schema": "jev.clef.synthetic-quality/v1", "fixture_version": 1,
              "provider": config["provider"], "model": config["model"], "synthetic": True,
              "input_kind": "text" if config["provider"] == "llamacpp" else "image",
              "evaluation_task": "literal_text_reading" if config["provider"] == "llamacpp" else "synthetic_shape_vision",
              "comparable_across_input_kinds": False,
              "benchmark_limitation": (
                  "Text-only sanity check: the input states the color and shape; these metrics do not measure vision and cannot be compared with image runs."
                  if config["provider"] == "llamacpp" else
                  "Vision over this synthetic shape family only; metrics cannot be compared with the text-only sanity check or generalized to production."),
              "max_requests": config["max_requests"], "reserved_requests": 0,
              "retries": 0, "passed": False}
    proof = smoke.provenance.Session(config, execution_observed=runner is smoke.bounded_run)
    config["jev"] = proof.executable
    if not proof.ready:
        result["reserved_requests"] = 0
        result["failure"] = "Provenance refused; check the named CLI, manifest, and local artifact files; nothing was sent"
        result["provenance"] = proof.finish()
        smoke.write_json(config["report"], result)
        return result
    try:
        with tempfile.TemporaryDirectory(prefix="jev-clef-quality-") as temporary:
            directory = Path(temporary)
            smoke.write_json(directory / "questions.json", {"questions": QUESTIONS})
            (directory / "dataset.jsonl").write_text("".join(json.dumps(row) + "\n" for row in expected), encoding="utf-8")
            try:
                result["reserved_requests"] = ROW_COUNT
                completed = runner(smoke.base_command(config) + ["eval", "--request", "questions.json",
                    "--dataset", "dataset.jsonl", "--no-split", "--show-rows", "--concurrency", "1", "--fail-fast"],
                    cwd=directory, env=smoke.environment(config), capture_output=True, text=True,
                    timeout=config.get("timeout", 120) * ROW_COUNT + 10)
                smoke.require(completed.returncode == 0, "quality CLI failed")
                result["quality"] = measure(smoke.json_document(completed.stdout), expected)
                result["passed"] = True
            except (smoke.Failure, OSError, ValueError, KeyError, TypeError, smoke.subprocess.SubprocessError):
                result["failure"] = "Quality integration failed; no retry or fallback was sent"
    except (OSError, ValueError):
        result["passed"] = False
        result["failure"] = "Quality fixture or cleanup failed; inspect the requested report"
    result["provenance"] = proof.finish()
    if proof.execution_observed and not proof.stable:
        result["passed"] = False
        result["failure"] = "Provenance changed or became unavailable during execution"
    smoke.write_json(config["report"], result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("plan", help="show the fixed original benchmark design offline")
    execute = commands.add_parser("run", help="explicitly send 32 labelled synthetic examples")
    execute.add_argument("--provider", choices=("cloudflare", "huggingface", "ollama", "llamacpp"), required=True)
    execute.add_argument("--model", required=True)
    execute.add_argument("--jev", required=True, help="explicit CLI path or executable name on PATH")
    execute.add_argument("--endpoint")
    execute.add_argument("--account")
    execute.add_argument("--key-file", type=Path)
    execute.add_argument("--max-requests", type=int, required=True)
    execute.add_argument("--timeout", type=int, default=120)
    execute.add_argument("--report", type=Path, required=True)
    execute.add_argument("--provenance-manifest", type=Path, help="explicit offline model/runtime file inventory to fingerprint")
    args = parser.parse_args()
    try:
        if args.command == "plan":
            print(json.dumps({"rows": ROW_COUNT, "positives": 8, "negatives": 24,
                "factors": {"shape": ["square", "circle"], "color": ["red", "blue"],
                            "left": [16, 56, 96, 136], "size": [40, 72]},
                "generalizable": False, "network": False}, indent=2))
            return 0
        config = vars(args)
        smoke.require(1 <= args.timeout <= 3600, "timeout must be between 1 and 3600 seconds")
        result = run(config)
        print(json.dumps({"passed": result["passed"], "reserved_requests": result["reserved_requests"]}))
        return 0 if result["passed"] else 1
    except (smoke.Failure, OSError, ValueError):
        print("Clef quality check refused or failed: check explicit arguments and the report.", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Opt-in, bounded Clef integration smoke checks using original synthetic inputs.

No model/credential discovery, dependency installation, or raw response logging.
The fixtures and plan commands are offline. Only run invokes the explicitly named CLI.
"""
from __future__ import annotations

import argparse
import base64
import io
import importlib.util
import ipaddress
import json
import math
import os
from pathlib import Path
import queue
import re
import struct
import subprocess
import sys
import tempfile
import threading
import time
from urllib.parse import urlsplit
import zlib

sys.dont_write_bytecode = True
provenance_spec = importlib.util.spec_from_file_location("clef_provenance", Path(__file__).with_name("clef_provenance.py"))
provenance = importlib.util.module_from_spec(provenance_spec)
provenance_spec.loader.exec_module(provenance)
TOOLS = ("noul", "choice", "score", "ask", "map")
LEVELS = ["No red square", "A red square is present"]
QUESTIONS = {
    "visible": {"type": "noul", "instructions": "Is a red square present?"},
    "color": {"type": "choice", "instructions": "What color is the main shape?",
              "criteria": {"red": "Red shape", "blue": "Blue shape"}},
    "presence": {"type": "score", "instructions": "Rate red square presence.",
                 "criteria": LEVELS},
}
STRUCTURED_QUESTIONS = {
    "visible": {"type": "noul", "instructions": {"task": "Is a red square present?"},
                "criteria": {"true": {"meaning": "Red square present"},
                             "false": ["No red square", "Other shapes do not qualify"]}},
    "color": {"type": "choice", "instructions": ["Identify the main shape color", "Use only the state"],
              "criteria": {"red": {"color": "red", "examples": ["red square"]},
                           "blue": ["blue shape", "blue circle"]}},
    "presence": {"type": "score", "instructions": {"task": "Rate red square presence", "levels": "Ordered"},
                 "criteria": [{"meaning": "No red square"}, ["A red square is present"]]},
}
MAX_CLI_OUTPUT = 4 * 1024 * 1024
MAX_EVAL_ROWS = 2


class Failure(ValueError):
    """A fixed diagnostic that never includes credentials or subprocess output."""


def require(condition, message):
    if not condition:
        raise Failure(message)


def png(positive=True, left=80):
    """Original 256px RGB shapes, encoded with the standard library alone."""
    pixels = bytearray()
    for y in range(256):
        pixels.append(0)
        for x in range(256):
            filled = (left <= x < left + 80 and 80 <= y < 160) if positive else (
                (x - 128) ** 2 + (y - 128) ** 2 < 45 ** 2)
            pixels.extend((230, 20, 20) if filled and positive else
                          (20, 20, 230) if filled else (255, 255, 255))
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 256, 256, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(bytes(pixels), 9)) + chunk(b"IEND", b""))


def fixtures(with_pillow=False):
    data = {"positive.png": png(), "negative.png": png(False),
            "frame-001.png": png(left=32), "frame-002.png": png(left=144)}
    missing = ["jpeg", "webp"]
    if with_pillow:
        try:
            from PIL import Image
        except ImportError:
            return data, missing
        for kind, suffix in (("JPEG", "jpg"), ("WEBP", "webp")):
            try:
                with Image.open(io.BytesIO(data["positive.png"])) as image:
                    output = io.BytesIO()
                    image.save(output, format=kind, quality=95)
                    data["positive." + suffix] = output.getvalue()
                missing.remove(kind.lower())
            except (OSError, ValueError, KeyError):
                pass
    return data, missing


def image(data, name):
    kind = {"png": "png", "jpg": "jpeg", "webp": "webp"}[name.rsplit(".", 1)[1]]
    return {"content_type": "image/" + kind, "base64": base64.b64encode(data[name]).decode("ascii")}


def probability(value):
    require(type(value) in (int, float) and 0 <= value <= 1 and math.isfinite(value),
            "invalid probability")


def validate_answers(document, questions, provider=None):
    require(type(document) is dict, "evaluation must be an object")
    answers = document.get("answers")
    require(type(answers) is dict and set(answers) == set(questions), "answer ids differ")
    require(not document.get("missing_answers"), "answers are missing")
    for name, question in questions.items():
        answer = answers[name]
        require(type(answer) is dict and answer.get("type") == question["type"], "answer type differs")
        if question["type"] == "noul":
            probability(answer.get("noul"))
            continue
        probability(answer.get("confidence"))
        probabilities = answer.get("probabilities")
        keys = (set(question["criteria"]) if question["type"] == "choice" else
                {str(index) for index in range(len(question["criteria"]))})
        require(type(probabilities) is dict and set(probabilities) == keys, "distribution keys differ")
        for value in probabilities.values():
            probability(value)
        require(abs(sum(probabilities.values()) - 1) <= 0.02, "distribution does not sum to one")
        # The publisher rounds probabilities and derived answers independently to
        # four decimals. Runtime providers also expose three-decimal output; the
        # weighted-mean error grows with the level numbers, not just one scalar.
        rounding = 0.00005 if provider == "huggingface" else 0.0005
        highest = max(probabilities.values())
        # Only the explicit bridge follows the publisher's max-probability
        # confidence formula. Other providers preserve their own confidence.
        if provider == "huggingface":
            require(abs(answer["confidence"] - highest) <= 2 * rounding + 1e-12,
                    "publisher confidence differs from distribution")
        if question["type"] == "choice":
            require(answer.get("choice") in keys, "choice outside supplied options")
            require(highest - probabilities[answer["choice"]] <= 2 * rounding + 1e-12,
                    "choice differs from most probable options")
        else:
            score = answer.get("score")
            require(type(score) in (int, float) and math.isfinite(score)
                    and 0 <= score <= len(keys) - 1, "score outside supplied levels")
            require(answer.get("legend") == dict(enumerate_strings(question["criteria"])),
                    "score legend differs")
            mean = sum(int(level) * value for level, value in probabilities.items())
            tolerance = rounding * (1 + sum(int(level) for level in keys))
            require(abs(score - mean) <= tolerance + 1e-12, "score differs from weighted mean")


def enumerate_strings(values):
    return ((str(index), value) for index, value in enumerate(values))


def quality_observation(document, expected):
    predicted = document["answers"]["visible"]["noul"] >= 0.5
    return {"expected": expected, "predicted": predicted, "agreed": predicted == expected}


def write_json(path, value):
    # Report parents can be replaced during slow inference. Never follow a link
    # when creating the explicitly requested receipt or an internal fixture.
    path = provenance.named_path(str(path))
    directory = provenance.open_parent(path)
    try:
        descriptor = os.open(path.name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                             0o600, dir_fd=directory)
        with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
            json.dump(value, stream, indent=2, allow_nan=False)
            stream.write("\n")
    finally:
        os.close(directory)


def materialize(directory, data):
    for name, raw in data.items():
        (directory / name).write_bytes(raw)
    write_json(directory / "questions.json", {"questions": QUESTIONS})
    write_json(directory / "text.json", {"state": "The only shape is a red square.", "questions": QUESTIONS})
    write_json(directory / "object.json", {"state": {"shape": "red square"}, "questions": QUESTIONS})
    write_json(directory / "array.json", {"state": ["red square"], "questions": QUESTIONS})
    write_json(directory / "structured-questions.json", {
        "state": {"shape": "square", "color": "red"}, "questions": STRUCTURED_QUESTIONS})
    write_json(directory / "video.json", {"state": "Two original ordered frames.",
        "questions": {"movement": {"type": "noul", "instructions": "Does the red square move from left to right?"}},
        "videos": [{"frames": [image(data, "frame-001.png"), image(data, "frame-002.png")],
                    "metadata": {"fps": 2, "total_num_frames": 2, "frames_indices": [0, 1], "duration": 1}}]})
    rows = [{"id": name, "state": "Inspect only the supplied shape.", "images": [image(data, name + ".png")]}
            for name in ("positive", "negative")]
    (directory / "records.jsonl").write_text("".join(json.dumps(row) + "\n" for row in rows), encoding="utf-8")
    (directory / "dataset.jsonl").write_text("".join(json.dumps({**row, "schema": "jev.eval.row/v1",
        "labels": {"visible": index == 0, "color": "red" if index == 0 else "blue",
                   "presence": 1 if index == 0 else 0}}) + "\n" for index, row in enumerate(rows)), encoding="utf-8")
    text_rows = [{"id": name, "state": "The only shape is a " + shape + "."}
                 for name, shape in (("positive", "red square"), ("negative", "blue circle"))]
    (directory / "text-records.jsonl").write_text("".join(json.dumps(row) + "\n" for row in text_rows), encoding="utf-8")
    (directory / "text-dataset.jsonl").write_text("".join(json.dumps({**row, "schema": "jev.eval.row/v1",
        "labels": {"visible": index == 0, "color": "red" if index == 0 else "blue",
                   "presence": 1 if index == 0 else 0}}) + "\n" for index, row in enumerate(text_rows)), encoding="utf-8")


def cases(provider, data):
    result = []
    def case(name, args, kind="evaluation", requests=1, questions=QUESTIONS, expected=None):
        result.append({"name": name, "args": args, "kind": kind, "requests": requests,
                       "questions": questions, "expected": expected})
    state = ["--state", "The only shape is a red square."]
    case("noul", ["noul", QUESTIONS["visible"]["instructions"], *state], questions={"answer": QUESTIONS["visible"]})
    case("choice", ["choice", QUESTIONS["color"]["instructions"], "--option", "red", "--option", "blue", *state],
         questions={"answer": QUESTIONS["color"]})
    case("score", ["score", QUESTIONS["presence"]["instructions"], "--level", LEVELS[0], "--level", LEVELS[1], *state],
         questions={"answer": QUESTIONS["presence"]})
    for name in ("text", "object", "array"):
        case("ask-" + name, ["ask", "--request", name + ".json"])
    case("ask-structured-questions", ["ask", "--request", "structured-questions.json"],
         questions=STRUCTURED_QUESTIONS)
    if provider != "llamacpp":
        for name in ("positive.png", "negative.png", "positive.jpg", "positive.webp"):
            if name in data:
                case("image-" + name, ["--image", name, "ask", "--request", "questions.json", "--state", "Inspect the supplied shape."],
                     expected=name != "negative.png")
        case("two-images", ["--image", "positive.png", "--image", "negative.png", "ask", "--request", "questions.json",
                            "--state", "Two original shape images."])
        if provider == "cloudflare":
            case("image-only", ["--image", "positive.png", "ask", "--request", "questions.json", "--state", ""], expected=True)
        batch = ["map", "--request", "questions.json", "--input", "records.jsonl", "--state-field", "state",
                 "--id-field", "id", "--images-field", "images", "--concurrency", "1", "--fail-fast"]
        case("map-images", batch, "map", 2)
        case("eval-images", ["eval", "--request", "questions.json", "--dataset", "dataset.jsonl",
              "--no-split", "--show-rows", "--concurrency", "1", "--fail-fast"], "eval", 2)
        case("resume-first", batch + ["--output-file", "resume.jsonl"], "resume-first", 2)
        # Reserve both rows even for the expected zero-call resume. This bounds a
        # regression that accidentally re-sends them instead of trusting the skip.
        case("resume-skip", batch + ["--output-file", "resume.jsonl", "--resume"], "resume-skip", 2)
        if provider == "huggingface":
            case("ordered-video", ["--video-frame", "frame-001.png", "--video-frame", "frame-002.png",
                "--video-fps", "2",
                "--media-kwargs", '{"max_pixels":65536,"do_sample_frames":false}', "noul",
                "Does the red square move from left to right?", "--state", "Two ordered frames."],
                questions={"answer": {"type": "noul"}})
            case("sampled-video", ["--video-frame", "frame-001.png", "--video-frame", "frame-002.png",
                "--video-fps", "2", "--media-kwargs", '{"max_pixels":65536,"num_frames":2,"do_sample_frames":true}',
                "noul", "Does the red square move from left to right?", "--state", "Two sampled frames."],
                questions={"answer": {"type": "noul"}})
            case("state-token-budget", ["--max-state-tokens", "16", "noul", "Is a red square mentioned?",
                "--state", "The only shape is a red square. " * 32], questions={"answer": {"type": "noul"}})
            case("video-source-metadata", ["ask", "--request", "video.json"],
                questions={"movement": {"type": "noul"}})
    else:
        batch = ["map", "--request", "questions.json", "--input", "text-records.jsonl", "--state-field", "state",
                 "--id-field", "id", "--concurrency", "1", "--fail-fast"]
        case("map-text", batch, "map", 2)
        case("eval-text", ["eval", "--request", "questions.json", "--dataset", "text-dataset.jsonl",
              "--no-split", "--show-rows", "--concurrency", "1", "--fail-fast"], "eval", 2)
        case("resume-first", batch + ["--output-file", "resume.jsonl"], "resume-first", 2)
        # A correct resume skips both rows; reserve them to bound accidental resends.
        case("resume-skip", batch + ["--output-file", "resume.jsonl", "--resume"], "resume-skip", 2)
    for tool in TOOLS:
        case("mcp-" + tool, [], "mcp", 2 if tool == "map" else 1)
    return result


def configuration(config):
    require(config.get("provider") in ("cloudflare", "huggingface", "ollama", "llamacpp"), "select a Clef provider")
    require(bool(config.get("model")) and bool(config.get("jev")), "name the model and CLI explicitly")
    require(type(config.get("max_requests")) is int and 1 <= config["max_requests"] <= 100,
            "max-requests must be between 1 and 100")
    report = Path(config.get("report", ""))
    provenance.named_path(str(report))
    require(bool(config.get("report")) and not report.exists() and report.parent.is_dir(),
            "name a new report file in an existing directory")
    if config["provider"] == "cloudflare":
        require(bool(re.fullmatch(r"[a-fA-F0-9]{32}", config.get("account") or "")) and bool(config.get("key_file")),
                "Cloudflare requires an explicit account and private key file")
        require(not config.get("endpoint"), "Cloudflare uses its official endpoint")
    else:
        endpoint = urlsplit(config.get("endpoint") or "")
        try:
            loopback = ipaddress.ip_address(endpoint.hostname or "").is_loopback
        except ValueError:
            loopback = endpoint.hostname == "localhost"
        require(endpoint.scheme == "http" and loopback and not endpoint.username and not endpoint.password
                and endpoint.path in ("", "/") and not endpoint.query and not endpoint.fragment,
                "local smoke runs require an explicit HTTP loopback endpoint")
        require(not config.get("account") and not config.get("key_file"), "loopback smoke runs are anonymous")


def environment(config):
    result = {key: value for key, value in os.environ.items()
              if not key.startswith(("JEV_", "TYPESAFE_", "CLOUDFLARE_"))}
    result["JEV_NO_KEYCHAIN"] = "1"
    if config.get("key_file"):
        # Only jev opens the secret file. Its contents never enter this harness.
        result["JEV_CUSTOM_API_KEY_FILE"] = str(config["key_file"])
    return result


def base_command(config):
    result = [str(config["jev"]), "--no-config", "--provider", config["provider"], "--model", config["model"],
              "--retries", "0", "--timeout", str(config.get("timeout", 120)), "--output", "json"]
    if config["provider"] == "cloudflare":
        result += ["--cloudflare-account-id", config["account"]]
    else:
        result += ["--endpoint", config["endpoint"]]
    return result


def json_document(raw):
    def invalid(_):
        raise Failure("nonfinite JSON")
    try:
        value = json.loads(raw, parse_constant=invalid)
        require(type(value) is dict, "JSON output must be an object")
        return value
    except (ValueError, TypeError, RecursionError):
        raise Failure("invalid JSON output") from None


def validate_rows(rows, questions, provider=None):
    require(len(rows) == 2, "batch row count differs")
    for index, row in enumerate(rows):
        require(type(row) is dict and row.get("schema") == "jev.map.row/v1" and row.get("ok") is True
                and row.get("index") == index and row.get("id") == ("positive", "negative")[index]
                and row.get("attempts") == 1, "batch ordering, outcome, or attempt count differs")
        validate_answers(row, questions, provider)


def validate_summary(summary, resumed=0):
    require(type(summary) is dict and summary.get("schema") == "jev.map.summary/v1",
            "batch summary is missing")
    expected = {"total": 2, "evaluated": 2-resumed, "resumed": resumed,
                "succeeded": 2-resumed, "failed": 0, "complete": 2,
                "stopped_early": False, "interrupted": False}
    require(all(type(summary.get(key)) is type(value) and summary[key] == value
                for key, value in expected.items()), "batch summary totals or outcome differ")


def eval_dataset(directory, args):
    """Read only the explicit smoke case's selected dataset, before running the CLI."""
    require(args.count("--dataset") == 1, "eval dataset must be selected once")
    index = args.index("--dataset") + 1
    require(index < len(args), "eval dataset path is missing")
    with (directory / args[index]).open("rb") as stream:
        raw = stream.read(MAX_CLI_OUTPUT + 1)
    require(len(raw) <= MAX_CLI_OUTPUT, "eval dataset limit exceeded")
    rows = [json_document(line) for line in raw.splitlines() if line.strip()]
    require(1 <= len(rows) <= MAX_EVAL_ROWS, "eval dataset row count differs")
    return rows


def validate_eval(document, questions, dataset):
    """Check typed per-row integration results, without imposing an accuracy gate."""
    require(type(dataset) is list and 1 <= len(dataset) <= MAX_EVAL_ROWS,
            "eval dataset row count differs")
    require(type(questions) is dict and 1 <= len(questions) <= 64, "eval question set differs")
    require(type(document) is dict and document.get("schema") == "jev.eval/v1", "eval schema differs")
    count = len(dataset)
    totals = document.get("rows")
    require(type(totals) is dict and type(totals.get("evaluated")) is int
            and totals["evaluated"] == count and type(totals.get("failed")) is int
            and totals["failed"] == 0, "eval did not evaluate selected rows successfully")
    sections = document.get("questions")
    require(type(sections) is dict and set(sections) == set(questions), "eval question set differs")
    for expected in dataset:
        require(type(expected) is dict and expected.get("schema") == "jev.eval.row/v1"
                and type(expected.get("id")) is str and bool(expected["id"])
                and type(expected.get("labels")) is dict
                and set(expected["labels"]) == set(questions), "eval dataset labels differ")
    for name, question in questions.items():
        require(type(question) is dict and question.get("type") in ("noul", "choice", "score"),
                "eval question type differs")
        kind = question["type"]
        section = sections[name]
        require(type(section) is dict and section.get("type") == kind, "eval section type differs")
        require(type(section.get("n")) is int and section["n"] == count
                and type(section.get("labelled")) is int and section["labelled"] == count,
                "eval did not score selected labelled examples")
        rows = section.get("rows")
        require(type(rows) is list and len(rows) == count, "eval section rows differ")
        criteria = question.get("criteria")
        if kind == "choice":
            require(type(criteria) is dict and 2 <= len(criteria) <= 255, "eval Choice options differ")
        elif kind == "score":
            require(type(criteria) is list and 2 <= len(criteria) <= 255, "eval Score levels differ")
        for row, expected in zip(rows, dataset):
            require(type(row) is dict and row.get("id") == expected["id"], "eval row ids or ordering differ")
            label = expected["labels"][name]
            if kind == "noul":
                require(type(label) is bool, "eval Noul label differs")
            elif kind == "choice":
                require(type(label) is str and label in criteria, "eval Choice label differs")
            else:
                require(type(label) is int and 0 <= label < len(criteria), "eval Score label differs")
            require(type(row.get("label")) is type(label) and row["label"] == label, "eval labels differ")
            predicted = row.get("predicted")
            if kind == "noul":
                probability(predicted)
            elif kind == "choice":
                require(type(predicted) is str and predicted in criteria, "eval Choice prediction differs")
            else:
                require(type(predicted) in (int, float) and 0 <= predicted <= len(criteria)-1
                        and math.isfinite(predicted), "eval Score prediction differs")


def bounded_run(command, *, cwd, env, timeout, **_capture):
    """Capture bounded stdout; diagnostics are never read or retained by the harness."""
    deadline = time.monotonic() + timeout
    process = subprocess.Popen(command, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                               stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    messages = queue.Queue(maxsize=1)
    def read():
        data = bytearray()
        try:
            while len(data) <= MAX_CLI_OUTPUT:
                chunk = process.stdout.read1(min(65536, MAX_CLI_OUTPUT + 1-len(data)))
                if not chunk:
                    messages.put(bytes(data))
                    return
                data.extend(chunk)
            messages.put(Failure("CLI output limit exceeded"))
        except (OSError, ValueError):
            messages.put(Failure("cannot read CLI output"))
        finally:
            process.stdout.close()
    reader = threading.Thread(target=read, daemon=True)
    reader.start()
    try:
        try:
            captured = messages.get(timeout=max(0, deadline-time.monotonic()))
        except queue.Empty:
            raise subprocess.TimeoutExpired(command, timeout) from None
        if isinstance(captured, Failure):
            raise captured
        process.wait(timeout=max(0, deadline-time.monotonic()))
        return subprocess.CompletedProcess(command, process.returncode,
                                           captured.decode("utf-8"), "")
    finally:
        if process.poll() is None:
            process.kill()
        process.wait()
        reader.join(timeout=1)


class MCP:
    """Bounded newline JSON-RPC session; stderr is discarded, never copied to reports."""
    def __init__(self, command, env, cwd, timeout):
        self.timeout = timeout
        self.process = subprocess.Popen(command + ["mcp", "serve"], env=env, cwd=cwd,
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        self.messages = queue.Queue(maxsize=32)
        self.counter = 0
        def read():
            while True:
                try:
                    line = self.process.stdout.readline(2 * 1024 * 1024 + 1)
                except (OSError, ValueError):
                    return
                if not line or len(line) > 2 * 1024 * 1024:
                    self.messages.put(None)
                    return
                self.messages.put(line)
        threading.Thread(target=read, daemon=True).start()

    def request(self, method, params, *, requests=1):
        self.counter += 1
        self.send({"jsonrpc": "2.0", "id": self.counter, "method": method, "params": params})
        # Notifications are bounded too; an endless stream must not defeat the deadline.
        deadline = time.monotonic() + self.timeout * requests + 10
        for _ in range(32):
            try:
                raw = self.messages.get(timeout=max(0, deadline - time.monotonic()))
            except queue.Empty:
                raise Failure("MCP response timeout") from None
            require(raw is not None, "MCP ended or exceeded output limit")
            response = json_document(raw)
            if response.get("id") == self.counter:
                require("error" not in response and type(response.get("result")) is dict, "MCP request failed")
                return response["result"]
        raise Failure("too many MCP notifications")

    def send(self, value):
        self.process.stdin.write(json.dumps(value).encode() + b"\n")
        self.process.stdin.flush()

    def close(self):
        self.process.stdin.close()
        forced = False
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            forced = True
            self.process.kill()
            self.process.wait()
        self.process.stdout.close()
        return not forced and self.process.returncode == 0


def mcp_arguments(tool, data, provider):
    state = "Inspect the supplied red square." if provider != "llamacpp" else "The only shape is a red square."
    media = {"images": [image(data, "positive.png")]} if provider != "llamacpp" else {}
    questions = []
    for name, question in QUESTIONS.items():
        item = {"id": name, "type": question["type"], "instructions": question["instructions"]}
        if question["type"] == "choice":
            item["options"] = [{"name": name, "description": description}
                               for name, description in question["criteria"].items()]
        elif question["type"] == "score":
            item["levels"] = question["criteria"]
        questions.append(item)
    if tool == "ask":
        return {"state": state, **media, "questions": questions}, QUESTIONS
    if tool == "map":
        return {"records": [{"id": name, "state": state, **media} for name in ("positive", "negative")],
                "questions": questions, "concurrency": 1}, QUESTIONS
    question = next(item for item in questions if item["type"] == tool)
    return {"state": state, **media, **{key: value for key, value in question.items() if key != "type"}}, {
        question["id"]: QUESTIONS[question["id"]]}


def run_matrix(config, runner=bounded_run, mcp_factory=MCP):
    configuration(config)
    config = dict(config)
    if config.get("key_file"):
        config["key_file"] = Path(config["key_file"]).resolve()
    data, missing = fixtures(config.get("with_pillow", False))
    matrix = cases(config["provider"], data)
    planned = sum(case["requests"] for case in matrix)
    require(planned <= config["max_requests"], "the complete matrix exceeds max-requests; nothing was sent")
    result = {"schema": "jev.clef.smoke/v1", "provider": config["provider"], "model": config["model"],
              "synthetic": True, "retries": 0, "max_requests": config["max_requests"],
              "planned_requests": planned, "reserved_requests": 0, "passed": True,
              "skipped": [{"case": name, "reason": "optional Pillow encoder not requested or unavailable"}
                          for name in missing], "cases": [], "quality_observations": []}
    if config["provider"] != "huggingface":
        result["skipped"].append({"case": "ordered-video", "reason": "requires Python bridge"})
    if config["provider"] == "llamacpp":
        result["skipped"].append({"case": "images-and-image-eval", "reason": "llama.cpp Clef images unsupported"})
    proof = provenance.Session(config, execution_observed=runner is bounded_run and mcp_factory is MCP)
    config["jev"] = proof.executable
    if not proof.ready:
        result["passed"] = False
        result["failure"] = "Provenance refused; check the named CLI, manifest, and local artifact files; nothing was sent"
        result["provenance"] = proof.finish()
        write_json(config["report"], result)
        return result
    command, env = base_command(config), environment(config)
    timeout = config.get("timeout", 120)
    mcp = None
    try:
        with tempfile.TemporaryDirectory(prefix="jev-clef-smoke-") as temporary:
            directory = Path(temporary)
            materialize(directory, data)
            saved = None
            try:
                for case in matrix:
                    item = {"case": case["name"], "requests_reserved": case["requests"], "passed": False}
                    result["reserved_requests"] += case["requests"]
                    try:
                        kind = case["kind"]
                        if kind == "mcp":
                            if mcp is None:
                                mcp = mcp_factory(command, env, directory, timeout)
                                initialized = mcp.request("initialize", {"protocolVersion": "2025-06-18",
                                    "capabilities": {}, "clientInfo": {"name": "jev-clef-smoke", "version": "1"}})
                                require(initialized.get("protocolVersion") == "2025-06-18", "MCP version differs")
                                mcp.send({"jsonrpc":"2.0", "method":"notifications/initialized"})
                                listed = mcp.request("tools/list", {})
                                require(type(listed) is dict, "MCP tool listing differs")
                                tools = listed.get("tools")
                                require(type(tools) is list and all(type(tool) is dict for tool in tools),
                                        "MCP tool listing differs")
                                require({tool.get("name") for tool in tools} == set(TOOLS), "MCP tool set differs")
                            tool = case["name"].removeprefix("mcp-")
                            arguments, questions = mcp_arguments(tool, data, config["provider"])
                            response = mcp.request("tools/call", {"name": tool, "arguments": arguments},
                                                   requests=case["requests"])
                            require(type(response) is dict, "MCP tool response differs")
                            require(response.get("isError") is not True, "MCP tool rejected the request")
                            document = response.get("structuredContent")
                            content = response.get("content")
                            require(type(content) is list and all(type(block) is dict for block in content),
                                    "MCP tool content differs")
                            texts = [block.get("text") for block in content if block.get("type") == "text"]
                            require(type(document) is dict and len(texts) == 1 and json_document(texts[0]) == document,
                                    "MCP text and structured content differ")
                            if tool == "map":
                                require(document.get("schema") == "jev.mcp.map/v1", "MCP map schema differs")
                                validate_rows(document.get("rows", []), questions, config["provider"])
                                validate_summary(document.get("summary"))
                            else:
                                require(document.get("schema") == "jev.evaluation/v1", "MCP evaluation schema differs")
                                validate_answers(document, questions, config["provider"])
                        else:
                            dataset = eval_dataset(directory, case["args"]) if kind == "eval" else None
                            completed = runner(command + case["args"], cwd=directory, env=env,
                                               capture_output=True, text=True, timeout=timeout * max(1, case["requests"]) + 10)
                            require(completed.returncode == 0, "CLI returned a nonzero exit code")
                            require(len(completed.stdout) <= MAX_CLI_OUTPUT, "CLI output limit exceeded")
                            if kind.startswith("resume"):
                                validate_summary(json_document(completed.stdout),
                                                 resumed=2 if kind == "resume-skip" else 0)
                                with (directory / "resume.jsonl").open("rb") as stream:
                                    raw = stream.read(MAX_CLI_OUTPUT + 1)
                                require(len(raw) <= MAX_CLI_OUTPUT, "resume output limit exceeded")
                                if kind == "resume-first":
                                    validate_rows([json_document(line) for line in raw.splitlines()], QUESTIONS, config["provider"])
                                    saved = raw
                                else:
                                    require(raw == saved, "resume changed previously successful rows")
                            elif kind == "map":
                                documents = [json_document(line) for line in completed.stdout.splitlines()]
                                require(len(documents) == 3, "batch row and summary count differs")
                                validate_rows(documents[:2], QUESTIONS, config["provider"])
                                validate_summary(documents[2])
                            elif kind == "eval":
                                document = json_document(completed.stdout)
                                validate_eval(document, case["questions"], dataset)
                                visible = document["questions"]["visible"]["rows"]
                                observations = []
                                for row, expected in zip(visible, dataset):
                                    observations.append(quality_observation({"answers": {"visible": {
                                        "noul": row["predicted"]}}}, expected["labels"]["visible"]))
                                result["quality_evaluation"] = {"rows": len(dataset), "threshold_for_observation": 0.5,
                                    "generalizable": False, "observations": observations,
                                    "input_kind": "text" if config["provider"] == "llamacpp" else "image",
                                    "evaluation_task": "literal_text_reading" if config["provider"] == "llamacpp" else "synthetic_shape_vision",
                                    "comparable_across_input_kinds": False,
                                    "limitation": (
                                        "Text-only sanity check: the input states the color and shape; these metrics do not measure vision and cannot be compared with image runs."
                                        if config["provider"] == "llamacpp" else
                                        "Two original synthetic images only; no production calibration claim or comparison with literal text runs")}
                            else:
                                document = json_document(completed.stdout)
                                require(document.get("schema") == "jev.evaluation/v1", "evaluation schema differs")
                                validate_answers(document, case["questions"], config["provider"])
                                if case["expected"] is not None:
                                    result["quality_observations"].append({"case": case["name"],
                                        **quality_observation(document, case["expected"])})
                        item["passed"] = True
                    except (Failure, OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError):
                        # Diagnostics may contain credentials from an untrusted executable.
                        # Persist only our fixed classification, never its output or exception.
                        item["failure"] = "integration check failed; no further requests were sent"
                        result["passed"] = False
                    result["cases"].append(item)
                    if not item["passed"]:
                        break
            finally:
                if mcp is not None:
                    try:
                        clean = mcp.close()
                    except (OSError, ValueError, subprocess.SubprocessError):
                        clean = False
                    result["mcp"] = {"clean_exit": clean}
                    if not clean:
                        result["passed"] = False
    except (OSError, ValueError):
        result["passed"] = False
        result["failure"] = "Smoke fixture or cleanup failed; inspect the requested report"
    result["provenance"] = proof.finish()
    if proof.execution_observed and not proof.stable:
        result["passed"] = False
        result["failure"] = "Provenance changed or became unavailable during execution"
    write_json(config["report"], result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    generate = commands.add_parser("fixtures", help="write original fixtures offline")
    generate.add_argument("--output-directory", type=Path, required=True)
    generate.add_argument("--with-pillow", action="store_true")
    plan = commands.add_parser("plan", help="print case names and request budget without starting jev")
    plan.add_argument("--provider", choices=("cloudflare", "huggingface", "ollama", "llamacpp"), required=True)
    plan.add_argument("--with-pillow", action="store_true")
    run = commands.add_parser("run", help="explicitly send synthetic inference requests")
    run.add_argument("--jev", required=True, help="explicit CLI path or executable name on PATH")
    run.add_argument("--provider", choices=("cloudflare", "huggingface", "ollama", "llamacpp"), required=True)
    run.add_argument("--model", required=True)
    run.add_argument("--endpoint")
    run.add_argument("--account")
    run.add_argument("--key-file", type=Path)
    run.add_argument("--max-requests", type=int, required=True)
    run.add_argument("--timeout", type=int, default=120)
    run.add_argument("--report", type=Path, required=True)
    run.add_argument("--with-pillow", action="store_true")
    run.add_argument("--provenance-manifest", type=Path, help="explicit offline model/runtime file inventory to fingerprint")
    args = parser.parse_args()
    try:
        if args.command == "fixtures":
            args.output_directory.mkdir(exist_ok=False)
            data, missing = fixtures(args.with_pillow)
            materialize(args.output_directory, data)
            write_json(args.output_directory / "manifest.json", {"synthetic": True, "files": sorted(data), "missing_formats": missing})
        elif args.command == "plan":
            data, missing = fixtures(args.with_pillow)
            matrix = cases(args.provider, data)
            print(json.dumps({"planned_requests": sum(case["requests"] for case in matrix),
                "cases": [{"case": case["name"], "requests": case["requests"]} for case in matrix], "missing_formats": missing}, indent=2))
        else:
            config = vars(args)
            require(1 <= args.timeout <= 3600, "timeout must be between 1 and 3600 seconds")
            result = run_matrix(config)
            print(json.dumps({"passed": result["passed"], "reserved_requests": result["reserved_requests"],
                              "skipped": result["skipped"]}))
            return 0 if result["passed"] else 1
        return 0
    except (Failure, OSError, ValueError):
        print("Clef smoke refused or failed: check explicit arguments and the requested report.", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())

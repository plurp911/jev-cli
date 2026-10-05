#!/usr/bin/env python3
"""Deterministic smoke-harness checks; no credentials, weights, or network."""
from __future__ import annotations

import copy
import importlib.util
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).with_name("clef-live.py")

EVAL_DATASET = [
    {"schema":"jev.eval.row/v1", "id":"positive", "state":"red square",
     "labels":{"visible":True, "color":"red", "presence":1}},
    {"schema":"jev.eval.row/v1", "id":"negative", "state":"blue circle",
     "labels":{"visible":False, "color":"blue", "presence":0}},
]
EVAL_DOCUMENT = {"schema":"jev.eval/v1", "rows":{"evaluated":2, "failed":0}, "questions":{
    "visible":{"type":"noul", "n":2, "labelled":2, "rows":[
        {"id":"positive", "label":True, "predicted":0.9},
        {"id":"negative", "label":False, "predicted":0.1}]},
    "color":{"type":"choice", "n":2, "labelled":2, "rows":[
        {"id":"positive", "label":"red", "predicted":"red"},
        {"id":"negative", "label":"blue", "predicted":"blue"}]},
    "presence":{"type":"score", "n":2, "labelled":2, "rows":[
        {"id":"positive", "label":1, "predicted":0.8},
        {"id":"negative", "label":0, "predicted":0.2}]},
}}


class HarnessTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.harness = None
        if SCRIPT.is_file():
            spec = importlib.util.spec_from_file_location("clef_live", SCRIPT)
            cls.harness = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(cls.harness)

    def setUp(self):
        self.assertIsNotNone(self.harness, "the opt-in Clef harness is missing")

    def test_failed_run_captures_cli_hash_before_call_and_changed_bytes_after(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            cli = root / "private-provenance-path"
            cli.write_bytes(b"abc")
            invoked = []
            def runner(command, **kwargs):
                invoked.append(command[0])
                cli.write_bytes(b"xyz")
                return subprocess.CompletedProcess(command, 4, "private-canary", "private-canary")
            result = self.harness.run_matrix({"provider": "ollama", "model": "clef-flash",
                "jev": cli, "endpoint": "http://127.0.0.1:8787", "max_requests": 64,
                "report": root / "report.json"}, runner=runner)
            self.assertEqual(invoked, [str(cli)])
            receipt = result["provenance"]
            self.assertEqual(receipt["artifacts"][0]["before"]["sha256"], hashlib.sha256(b"abc").hexdigest())
            self.assertEqual(receipt["artifacts"][0]["status"], "changed")
            self.assertEqual(receipt["execution_boundary"], "injected_runner_unobserved")
            raw = (root / "report.json").read_text()
            self.assertNotIn("private-canary", raw)
            self.assertNotIn(str(cli), raw)

    def test_invalid_provenance_manifest_writes_failure_without_cli_or_auth_read(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest = root / "manifest.json"
            manifest.write_text('{"schema":"jev.clef.provenance-inputs/v1","artifacts":[{"role":"model","path":"missing"}]}')
            result = self.harness.run_matrix({"provider": "cloudflare", "model": "clef",
                "jev": "/stub", "account": "0" * 32, "key_file": root / "unopened-secret",
                "max_requests": 64, "report": root / "report.json", "provenance_manifest": manifest},
                runner=lambda *a, **kw: self.fail("invalid provenance must never invoke CLI"))
            self.assertFalse(result["passed"])
            self.assertEqual(result["reserved_requests"], 0)
            self.assertEqual(result["provenance"]["status"], "refused")

    def test_real_helpers_resolve_path_alias_and_report_zero_calls_for_bad_manifest(self):
        for script in (SCRIPT, SCRIPT.with_name("clef-quality.py")):
            with self.subTest(script=script.name), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                cli = root / "actual-cli"
                cli.write_text("#!/bin/sh\nexit 4\n", encoding="utf-8")
                cli.chmod(0o700)
                (root / "alias").symlink_to(cli)
                manifest = root / "manifest.json"
                manifest.write_text("{}")
                args = [sys.executable, str(script), "run", "--provider", "ollama", "--model", "clef-flash",
                        "--jev", "alias", "--endpoint", "http://127.0.0.1:8787", "--max-requests", "64"]
                env = dict(os.environ, PATH=str(root))
                bad_report = root / "bad-report.json"
                completed = subprocess.run([*args, "--report", str(bad_report),
                    "--provenance-manifest", str(manifest)], env=env, capture_output=True, text=True, check=False)
                self.assertEqual(completed.returncode, 1, completed.stderr)
                self.assertEqual(json.loads(completed.stdout)["reserved_requests"], 0)
                bad = json.loads(bad_report.read_text())
                self.assertEqual(bad["reserved_requests"], 0)
                self.assertEqual(bad["provenance"]["status"], "refused")
                report = root / "report.json"
                completed = subprocess.run([*args, "--report", str(report)], env=env,
                                           capture_output=True, text=True, check=False)
                self.assertEqual(completed.returncode, 1, completed.stderr)
                result = json.loads(report.read_text())
                cli_hash = result["provenance"]["artifacts"][0]["before"]["sha256"]
                self.assertEqual(cli_hash, hashlib.sha256(cli.read_bytes()).hexdigest())
                self.assertEqual(result["provenance"]["execution_boundary"], "resolved_subprocess_target")
                self.assertEqual(result["provenance"]["artifacts"][0]["status"], "unchanged")

    def test_real_helpers_reject_deep_cli_json_with_failure_receipts(self):
        for script in (SCRIPT, SCRIPT.with_name("clef-quality.py")):
            with self.subTest(script=script.name), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                cli = root / "deep-cli"
                cli.write_text("#!" + sys.executable + "\nprint('{\"x\":'+'['*10000+'0'+']'*10000+'}')\n")
                cli.chmod(0o700)
                report = root / "report.json"
                run = subprocess.run([sys.executable, str(script), "run", "--provider", "ollama",
                    "--model", "clef-flash", "--jev", str(cli), "--endpoint", "http://127.0.0.1:8787",
                    "--max-requests", "64", "--report", str(report)], capture_output=True, text=True)
                self.assertEqual(run.returncode, 1)
                self.assertNotIn("Traceback", run.stderr)
                value = json.loads(report.read_text())
                self.assertFalse(value["passed"])
                self.assertEqual(value["provenance"]["status"], "stable")

    def test_mcp_close_failure_still_writes_sanitized_provenance_receipt(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            stub = root / "stub.py"
            stub.write_text(STUB, encoding="utf-8")
            def runner(command, **kwargs):
                return self.harness.bounded_run([sys.executable, str(stub), *command[1:]], **kwargs)
            class BrokenClose(self.harness.MCP):
                def close(self):
                    super().close()
                    raise OSError("private-close-canary")
            def mcp(command, env, cwd, timeout):
                return BrokenClose([sys.executable, str(stub), *command[1:]], env, cwd, timeout)
            result = self.harness.run_matrix({"provider": "llamacpp", "model": "clef-flash",
                "jev": "/stub", "endpoint": "http://127.0.0.1:8080", "max_requests": 64,
                "report": root / "report.json"}, runner=runner, mcp_factory=mcp)
            self.assertFalse(result["passed"])
            self.assertFalse(result["mcp"]["clean_exit"])
            self.assertIn("provenance", result)
            self.assertNotIn("private-close-canary", (root / "report.json").read_text())

    def test_both_real_helpers_refuse_hostile_cli_names_without_tracebacks(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            loop = root / "loop"
            loop.symlink_to(loop)
            for script in (SCRIPT, SCRIPT.with_name("clef-quality.py")):
                for index, selected in enumerate(("", str(loop), "x" * 5000, "private\ncanary")):
                    with self.subTest(script=script.name, selected=index):
                        report = root / (script.stem + str(index) + ".json")
                        completed = subprocess.run([sys.executable, str(script), "run", "--provider", "ollama",
                            "--model", "clef-flash", "--jev", selected, "--endpoint", "http://127.0.0.1:8787",
                            "--max-requests", "64", "--report", str(report)], capture_output=True, text=True, check=False)
                        self.assertNotIn("Traceback", completed.stdout + completed.stderr)
                        self.assertNotIn("private", completed.stdout + completed.stderr)
                        self.assertNotEqual(completed.returncode, 0)

    def test_fixture_creation_failure_preserves_provenance_without_calls(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with patch.object(self.harness, "materialize", side_effect=OSError("private-canary")):
                result = self.harness.run_matrix({"provider": "ollama", "model": "clef-flash", "jev": "/stub",
                    "endpoint": "http://127.0.0.1:8787", "max_requests": 64, "report": root / "report.json"},
                    runner=lambda *args, **kw: self.fail("must not run"))
            self.assertFalse(result["passed"])
            self.assertEqual(result["reserved_requests"], 0)
            self.assertIn("provenance", result)
            self.assertNotIn("private-canary", (root / "report.json").read_text())

    def test_original_png_fixtures_are_deterministic_and_distinct(self):
        first, missing = self.harness.fixtures(False)
        second, _ = self.harness.fixtures(False)
        self.assertEqual(first, second)
        self.assertEqual(set(missing), {"jpeg", "webp"})
        self.assertTrue(first["positive.png"].startswith(b"\x89PNG\r\n\x1a\n"))
        self.assertNotEqual(first["positive.png"], first["negative.png"])
        self.assertNotEqual(first["frame-001.png"], first["frame-002.png"])

    def test_missing_cloudflare_configuration_refuses_before_invoking_cli(self):
        with tempfile.TemporaryDirectory() as temporary:
            report = Path(temporary) / "report.json"
            with self.assertRaises(self.harness.Failure):
                self.harness.run_matrix({"provider": "cloudflare", "model": "clef",
                    "jev": "/not/invoked", "max_requests": 64, "report": report},
                    runner=lambda *args, **kwargs: self.fail("CLI must not run"))
            self.assertFalse(report.exists())

    def test_real_harness_clis_refuse_missing_provider_configuration_without_invoking_cli(self):
        for script in (SCRIPT, SCRIPT.with_name("clef-quality.py")):
            for provider in ("cloudflare", "huggingface", "ollama", "llamacpp"):
                with self.subTest(script=script.name, provider=provider), tempfile.TemporaryDirectory() as temporary:
                    root = Path(temporary)
                    marker = root / "invoked"
                    cli = root / "cli"
                    cli.write_text("#!/bin/sh\ntouch '" + str(marker) + "'\n", encoding="utf-8")
                    cli.chmod(0o700)
                    report = root / "report.json"
                    env = {name: value for name, value in os.environ.items()
                           if not name.startswith(("JEV_", "TYPESAFE_", "CLOUDFLARE_"))}
                    env["JEV_API_KEY"] = "private-refusal-canary"
                    result = subprocess.run([sys.executable, str(script), "run", "--provider", provider,
                        "--model", "clef-flash", "--jev", str(cli), "--max-requests", "64",
                        "--report", str(report)], env=env, capture_output=True, text=True, check=False)
                    self.assertEqual(result.returncode, 2, result.stderr)
                    self.assertEqual(result.stdout, "")
                    self.assertIn("refused", result.stderr)
                    self.assertNotIn("Traceback", result.stderr)
                    self.assertNotIn("private-refusal-canary", result.stdout + result.stderr)
                    self.assertFalse(marker.exists(), "refused configuration must never invoke the CLI")
                    self.assertFalse(report.exists())

    def test_request_budget_is_checked_before_first_cli_process(self):
        with tempfile.TemporaryDirectory() as temporary:
            with self.assertRaises(self.harness.Failure):
                self.harness.run_matrix({"provider": "huggingface", "model": "clef-flash",
                    "jev": "/stub", "endpoint": "http://127.0.0.1:8787", "max_requests": 1,
                    "report": Path(temporary) / "report.json"},
                    runner=lambda *args, **kwargs: self.fail("CLI must not run"))

    def test_nonloopback_local_endpoint_and_existing_report_are_refused(self):
        with tempfile.TemporaryDirectory() as temporary:
            report = Path(temporary) / "report.json"
            report.write_text("preserve me")
            for endpoint in ("https://remote.example", "http://127.0.0.1:8787"):
                with self.subTest(endpoint=endpoint), self.assertRaises(self.harness.Failure):
                    self.harness.run_matrix({"provider": "huggingface", "model": "clef-flash",
                        "jev": "/stub", "endpoint": endpoint, "max_requests": 64,
                        "report": report}, runner=lambda *a, **kw: self.fail("CLI must not run"))
            self.assertEqual(report.read_text(), "preserve me")

    def test_failures_stop_sending_and_never_copy_cli_diagnostics_into_report(self):
        with tempfile.TemporaryDirectory() as temporary:
            report = Path(temporary) / "report.json"
            result = self.harness.run_matrix({"provider": "huggingface", "model": "clef-flash",
                "jev": "/stub", "endpoint": "http://127.0.0.1:8787", "max_requests": 64,
                "report": report}, runner=lambda *a, **kw: subprocess.CompletedProcess(
                    a, 4, "canary-private-value", "Bearer canary-private-value"))
            self.assertFalse(result["passed"])
            self.assertEqual(result["reserved_requests"], 1)
            self.assertEqual(len(result["cases"]), 1)
            self.assertNotIn("canary-private-value", report.read_text())

    def test_answer_checks_fail_missing_ids_bad_probabilities_and_wrong_legend(self):
        good = {"schema":"jev.evaluation/v1", "answers":{
            "color":{"type":"choice", "choice":"red", "confidence":0.8,
                "probabilities":{"red":0.8,"blue":0.2}}}}
        questions = {"color":{"type":"choice", "criteria":{"red":None,"blue":None}}}
        self.harness.validate_answers(good, questions)
        for bad in ({"answers":{}}, {"answers":{"color":{
                "type":"choice","choice":"red","confidence":0.8,
                "probabilities":{"red":0.8,"blue":0.8}}}}):
            with self.subTest(bad=bad), self.assertRaises(self.harness.Failure):
                self.harness.validate_answers(bad, questions)

    def test_wrong_json_output_type_is_an_opaque_recorded_failure(self):
        with tempfile.TemporaryDirectory() as temporary:
            report = Path(temporary) / "report.json"
            try:
                result = self.harness.run_matrix({"provider":"huggingface", "model":"clef-flash",
                    "jev":"/stub", "endpoint":"http://127.0.0.1:8787", "max_requests":64,
                    "report":report}, runner=lambda *a, **kw: subprocess.CompletedProcess(a, 0, '[]', ''))
            except AttributeError:
                self.fail("malformed output escaped the harness failure guard")
            self.assertFalse(result["passed"])
            self.assertEqual(len(result["cases"]), 1)
            self.assertTrue(report.is_file())

    def test_quality_observations_do_not_turn_model_disagreement_into_transport_failure(self):
        answer = {"answers":{"visible":{"type":"noul","noul":0.1}}}
        observation = self.harness.quality_observation(answer, True)
        self.assertEqual(observation, {"expected":True,"predicted":False,"agreed":False})

    def test_answer_checks_reject_swapped_choice_and_wrong_score_mean(self):
        choice = {"type":"choice", "choice":"blue", "confidence":0.8,
                  "probabilities":{"red":0.8, "blue":0.2}}
        score = {"type":"score", "score":0.1, "confidence":0.8,
                 "probabilities":{"0":0.2, "1":0.8}, "legend":{"0":"low", "1":"high"}}
        for answer, criteria in ((choice, {"red":None,"blue":None}), (score, ["low","high"])):
            with self.subTest(kind=answer["type"]), self.assertRaises(self.harness.Failure):
                self.harness.validate_answers({"answers":{"q":answer}},
                    {"q":{"type":answer["type"],"criteria":criteria}})

    def test_publisher_answer_checks_reject_wrong_choice_and_score_confidence(self):
        for kind, criteria, fields in (
            ("choice", {"red":None,"blue":None}, {"choice":"red", "probabilities":{"red":0.8,"blue":0.2}}),
            ("score", ["low","high"], {"score":0.8,"probabilities":{"0":0.2,"1":0.8},
                                      "legend":{"0":"low","1":"high"}})):
            answer = {"type":kind,"confidence":0.1,**fields}
            with self.subTest(kind=kind), self.assertRaises(self.harness.Failure):
                self.harness.validate_answers({"answers":{"q":answer}},
                    {"q":{"type":kind,"criteria":criteria}}, provider="huggingface")

    def test_real_cli_and_mcp_output_with_wrong_publisher_confidence_fails_smoke(self):
        for name in ("choice", "mcp-choice"):
            with self.subTest(case=name), tempfile.TemporaryDirectory() as temporary:
                stub = Path(temporary) / "stub.py"
                stub.write_text(STUB.replace("'confidence':0.8", "'confidence':0.1"), encoding="utf-8")
                case = next(case for case in self.harness.cases("huggingface", {}) if case["name"] == name)
                def runner(command, **kwargs):
                    return self.harness.bounded_run([sys.executable, str(stub), *command[1:]], **kwargs)
                def mcp(command, env, cwd, timeout):
                    return self.harness.MCP([sys.executable, str(stub), *command[1:]], env, cwd, timeout)
                with patch.object(self.harness, "cases", return_value=[case]):
                    result = self.harness.run_matrix({"provider":"huggingface", "model":"clef-flash",
                        "jev":"/stub", "endpoint":"http://127.0.0.1:8787", "max_requests":64,
                        "report":Path(temporary)/"report.json"}, runner=runner, mcp_factory=mcp)
                self.assertFalse(result["passed"], "inconsistent confidence must fail integration")
                self.assertEqual(result["cases"][0]["failure"],
                                 "integration check failed; no further requests were sent")

    def test_answer_checks_preserve_rounding_ties_and_provider_specific_confidence(self):
        questions = {"q":{"type":"choice","criteria":{"red":None,"blue":None}}}
        for selected in ("red", "blue"):
            answer = {"type":"choice","choice":selected,"confidence":0.5,
                      "probabilities":{"red":0.5,"blue":0.5}, "future_field":True}
            self.harness.validate_answers({"answers":{"q":answer}}, questions, provider="huggingface")
        # TypeSafe and runtime providers can compute confidence differently from the publisher.
        answer = {"type":"choice","choice":"red","confidence":0.6,
                  "probabilities":{"red":0.8,"blue":0.2}}
        for provider in (None, "cloudflare", "ollama", "llamacpp"):
            self.harness.validate_answers({"answers":{"q":answer}}, questions, provider=provider)
        score = {"type":"score","score":1.888,"confidence":0.333,
                 "probabilities":{"0":0.111,"1":0.222,"2":0.333,"3":0.333},
                 "legend":{"0":"a","1":"b","2":"c","3":"d"}}
        self.harness.validate_answers({"answers":{"q":score}},
            {"q":{"type":"score","criteria":["a","b","c","d"]}})

    def test_publisher_score_rounding_budget_scales_to_255_levels(self):
        # Uniform 1/255 rounds to .0039 at every level; the exact mean is 127,
        # while the sum using independently rounded probabilities is 126.3015.
        levels = ["level " + str(index) for index in range(255)]
        score = {"type":"score","score":127,"confidence":0.0039,
                 "probabilities":{str(index):0.0039 for index in range(255)},
                 "legend":{str(index):value for index, value in enumerate(levels)}}
        self.harness.validate_answers({"answers":{"q":score}},
            {"q":{"type":"score","criteria":levels}}, provider="huggingface")
        score["score"] = 120
        with self.assertRaises(self.harness.Failure):
            self.harness.validate_answers({"answers":{"q":score}},
                {"q":{"type":"score","criteria":levels}}, provider="huggingface")

    def test_noul_uses_only_the_documented_probability_and_unknown_types_are_refused(self):
        question = {"q":{"type":"noul"}}
        self.harness.validate_answers({"answers":{"q":{"type":"noul","noul":0.5}}}, question)
        for value in (True, False, "true", None, float("nan"), float("inf"), -0.1, 1.1):
            with self.subTest(noul=value), self.assertRaises(self.harness.Failure):
                self.harness.validate_answers({"answers":{"q":{"type":"noul","noul":value}}}, question)
        with self.assertRaises(self.harness.Failure):
            self.harness.validate_answers({"answers":{"q":{"type":"unknown","noul":0.5}}}, question)

    def eval_smoke(self, document, dataset=None):
        """Exercise the real eval branch with a controlled, network-free CLI boundary."""
        materialize = self.harness.materialize
        def prepare(directory, data):
            materialize(directory, data)
            if dataset is not None:
                (directory / "text-dataset.jsonl").write_text(
                    "".join(json.dumps(row) + "\n" for row in dataset), encoding="utf-8")
        case = next(case for case in self.harness.cases("llamacpp", {})
                    if case["name"] == "eval-text")
        with tempfile.TemporaryDirectory() as temporary:
            with patch.object(self.harness, "cases", return_value=[case]), \
                    patch.object(self.harness, "materialize", side_effect=prepare):
                return self.harness.run_matrix({"provider":"llamacpp", "model":"clef-flash",
                    "jev":"/stub", "endpoint":"http://127.0.0.1:8080", "max_requests":2,
                    "report":Path(temporary)/"report.json"}, runner=lambda command, **kwargs:
                    subprocess.CompletedProcess(command, 0, json.dumps(document), ""))

    def test_eval_rejects_wrong_sections_labels_predictions_and_order_for_every_question(self):
        self.assertTrue(self.eval_smoke(EVAL_DOCUMENT)["passed"])
        for name in ("visible", "color", "presence"):
            for field, value in [("type", None), ("type", "other"), ("n", True),
                                 ("labelled", True), ("rows", None)]:
                bad = copy.deepcopy(EVAL_DOCUMENT)
                bad["questions"][name][field] = value
                with self.subTest(name=name, field=field, value=value):
                    self.assertFalse(self.eval_smoke(bad)["passed"])
            for field, value in [("id", "unknown"), ("label", None), ("predicted", None),
                                 ("label", "wrong-label"), ("predicted", True)]:
                bad = copy.deepcopy(EVAL_DOCUMENT)
                bad["questions"][name]["rows"][0][field] = value
                with self.subTest(name=name, field=field, value=value):
                    self.assertFalse(self.eval_smoke(bad)["passed"])
            bad = copy.deepcopy(EVAL_DOCUMENT)
            bad["questions"][name]["rows"].reverse()
            with self.subTest(name=name, field="ordering"):
                self.assertFalse(self.eval_smoke(bad)["passed"])
        for name, value in [("visible", -0.01), ("visible", 1.01), ("visible", "0.9"),
                            ("color", "green"), ("color", 0), ("presence", -0.01),
                            ("presence", 1.01), ("presence", "0.8")]:
            bad = copy.deepcopy(EVAL_DOCUMENT)
            bad["questions"][name]["rows"][0]["predicted"] = value
            with self.subTest(name=name, predicted=value):
                self.assertFalse(self.eval_smoke(bad)["passed"])
        for name, value in [("visible", 1), ("color", True), ("presence", True), ("presence", 1.0)]:
            bad = copy.deepcopy(EVAL_DOCUMENT)
            bad["questions"][name]["rows"][0]["label"] = value
            with self.subTest(name=name, label=value):
                self.assertFalse(self.eval_smoke(bad)["passed"])

    def test_eval_compares_against_the_selected_dataset_and_keeps_quality_observational(self):
        dataset = copy.deepcopy(EVAL_DATASET)
        document = copy.deepcopy(EVAL_DOCUMENT)
        for index, row in enumerate(dataset):
            row["id"] = "selected-" + str(index)
            row["labels"] = {"visible":index != 0, "color":"blue" if index == 0 else "red",
                             "presence":index}
            for name, section in document["questions"].items():
                section["rows"][index]["id"] = row["id"]
                section["rows"][index]["label"] = row["labels"][name]
        result = self.eval_smoke(document, dataset)
        self.assertTrue(result["passed"], result["cases"])
        self.assertEqual(result["quality_evaluation"]["observations"], [
            {"expected":False,"predicted":True,"agreed":False},
            {"expected":True,"predicted":False,"agreed":False}])
        self.assertFalse(self.eval_smoke(EVAL_DOCUMENT, dataset)["passed"])

    def test_eval_validator_bounds_rows_and_refuses_malformed_or_nonfinite_values(self):
        self.harness.validate_eval(EVAL_DOCUMENT, self.harness.QUESTIONS, EVAL_DATASET)
        with self.assertRaises(self.harness.Failure):
            self.harness.validate_eval(EVAL_DOCUMENT, self.harness.QUESTIONS, EVAL_DATASET * 2)
        for name in ("visible", "color", "presence"):
            for value in (None, [], True, "wrong-shape"):
                bad = copy.deepcopy(EVAL_DOCUMENT)
                bad["questions"][name] = value
                with self.subTest(name=name, shape=value), self.assertRaises(self.harness.Failure):
                    self.harness.validate_eval(bad, self.harness.QUESTIONS, EVAL_DATASET)
        for name in ("visible", "presence"):
            for value in (float("nan"), float("inf"), -float("inf"), 10**400):
                bad = copy.deepcopy(EVAL_DOCUMENT)
                bad["questions"][name]["rows"][0]["predicted"] = value
                with self.subTest(name=name, prediction=value), self.assertRaises(self.harness.Failure):
                    self.harness.validate_eval(bad, self.harness.QUESTIONS, EVAL_DATASET)
        for name in ("visible", "color", "presence"):
            bad = copy.deepcopy(EVAL_DOCUMENT)
            bad["questions"][name]["rows"][0] = None
            with self.subTest(name=name, missing_row=True), self.assertRaises(self.harness.Failure):
                self.harness.validate_eval(bad, self.harness.QUESTIONS, EVAL_DATASET)

    def test_eval_dataset_is_read_before_cli_and_bounded_to_the_reserved_rows(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            selected = root / "selected.jsonl"
            other = root / "dataset.jsonl"
            selected.write_text("".join(json.dumps(row) + "\n" for row in EVAL_DATASET), encoding="utf-8")
            other.write_text("not selected", encoding="utf-8")
            self.assertEqual(self.harness.eval_dataset(root, ["--dataset", "selected.jsonl"]), EVAL_DATASET)
            selected.write_text("{}\n" * 3, encoding="utf-8")
            with self.assertRaises(self.harness.Failure):
                self.harness.eval_dataset(root, ["--dataset", "selected.jsonl"])
            with patch.object(self.harness, "MAX_CLI_OUTPUT", 8), self.assertRaises(self.harness.Failure):
                self.harness.eval_dataset(root, ["--dataset", "selected.jsonl"])

    def test_each_provider_exercises_structured_instructions_and_all_criteria_types(self):
        data, _ = self.harness.fixtures(False)
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            self.harness.materialize(directory, data)
            for provider in ("cloudflare", "huggingface", "ollama", "llamacpp"):
                with self.subTest(provider=provider):
                    matching = [case for case in self.harness.cases(provider, data)
                                if case["name"] == "ask-structured-questions"]
                    self.assertEqual(len(matching), 1)
                    case = matching[0]
                    request = json.loads((directory / case["args"][2]).read_text())
                    self.assertEqual(request["questions"], case["questions"])
                    self.assertEqual({q["type"] for q in case["questions"].values()},
                                     {"noul", "choice", "score"})
                    descriptions = []
                    for question in case["questions"].values():
                        self.assertIsInstance(question["instructions"], (dict, list))
                        criteria = question["criteria"]
                        descriptions.extend(criteria.values() if isinstance(criteria, dict) else criteria)
                    self.assertTrue(any(isinstance(value, dict) for value in descriptions))
                    self.assertTrue(any(isinstance(value, list) for value in descriptions))

    def test_resume_budget_reserves_both_rows_even_if_regression_resends_them(self):
        data, _ = self.harness.fixtures(False)
        cases = self.harness.cases("cloudflare", data)
        resume = next(case for case in cases if case["name"] == "resume-skip")
        self.assertEqual(resume["requests"], 2)

    def test_cli_capture_discards_non_utf8_stderr(self):
        self.assertTrue(callable(getattr(self.harness, "bounded_run", None)), "bounded CLI capture is missing")
        completed = self.harness.bounded_run(
            [sys.executable, "-c", "import os; os.write(2,b'\\xff'); print('{}')"],
            cwd=None, env=None, capture_output=True, text=True, timeout=5)
        self.assertEqual(completed.stdout, "{}\n")
        self.assertEqual(completed.stderr, "")

    def test_cli_capture_kills_and_closes_process_when_stdout_exceeds_limit(self):
        self.assertTrue(callable(getattr(self.harness, "bounded_run", None)), "bounded CLI capture is missing")
        processes = []
        original = subprocess.Popen
        def start(*args, **kwargs):
            process = original(*args, **kwargs)
            processes.append(process)
            return process
        with patch.object(self.harness.subprocess, "Popen", side_effect=start):
            with self.assertRaises(self.harness.Failure) as caught:
                self.harness.bounded_run(
                    [sys.executable, "-c", "import os; os.write(1,b'x'*(5*1024*1024));\nwhile True: pass"],
                    cwd=None, env=None, capture_output=True, text=True, timeout=5)
        self.assertIn("output limit", str(caught.exception))
        self.assertIsNotNone(processes[0].returncode)
        self.assertTrue(processes[0].stdout.closed)

    def test_mcp_map_can_wait_for_two_sequential_requests(self):
        session = self.harness.MCP.__new__(self.harness.MCP)
        session.timeout, session.counter = 2, 0
        session.send = lambda value: None
        class ResponseAfterFirstRequest:
            def get(self, timeout):
                if timeout <= 12:
                    raise self.harness.queue.Empty
                return b'{"jsonrpc":"2.0","id":1,"result":{"finished":true}}'
        messages = ResponseAfterFirstRequest()
        messages.harness = self.harness
        session.messages = messages
        try:
            response = session.request("tools/call", {}, requests=2)
        except TypeError:
            self.fail("MCP cannot reserve a deadline for two sequential requests")
        self.assertEqual(response, {"finished":True})

    def test_relative_credential_file_is_resolved_before_fixture_cwd(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            before = Path.cwd()
            seen = []
            def runner(command, **kwargs):
                seen.append(kwargs["env"]["JEV_CUSTOM_API_KEY_FILE"])
                return subprocess.CompletedProcess(command, 4, "", "")
            try:
                os.chdir(root)
                self.harness.run_matrix({"provider":"cloudflare", "model":"clef-flash",
                    "jev":"/stub", "account":"0"*32, "key_file":Path("named-token-file"),
                    "max_requests":64, "report":root/"report.json"}, runner=runner)
            finally:
                os.chdir(before)
            self.assertEqual(seen, [str(root/"named-token-file")])

    def test_batch_summary_refuses_partial_failed_and_spurious_resumed_totals(self):
        good = {"schema":"jev.map.summary/v1", "total":2, "evaluated":2, "resumed":0,
                "succeeded":2, "failed":0, "complete":2, "stopped_early":False, "interrupted":False}
        self.harness.validate_summary(good)
        for key, value in (("total", 1), ("failed", 1), ("resumed", 2),
                           ("complete", 1), ("stopped_early", True), ("interrupted", True)):
            with self.subTest(key=key), self.assertRaises(self.harness.Failure):
                self.harness.validate_summary({**good, key:value})
        self.harness.validate_summary({**good, "resumed":2, "evaluated":0, "succeeded":0}, resumed=2)

    def test_llamacpp_plan_reserves_text_batch_cases_without_changing_other_provider_budgets(self):
        data, _ = self.harness.fixtures(False)
        for provider, expected in (("llamacpp",21), ("huggingface",28), ("cloudflare",25), ("ollama",24)):
            with self.subTest(provider=provider):
                matrix = self.harness.cases(provider, data)
                self.assertEqual(sum(case["requests"] for case in matrix), expected)
        matrix = self.harness.cases("llamacpp", data)
        self.assertTrue({"map-text", "eval-text", "resume-first", "resume-skip"} <= {case["name"] for case in matrix})
        self.assertFalse(any("--images-field" in case["args"] or "--image" in case["args"]
                             or "--video-frame" in case["args"] for case in matrix))

    def test_llamacpp_full_offline_matrix_uses_text_rows_and_reports_unsupported_media(self):
        with tempfile.TemporaryDirectory() as temporary:
            stub = Path(temporary) / "stub.py"
            stub.write_text(STUB, encoding="utf-8")
            checked = []
            def runner(command, **kwargs):
                for flag in ("--input", "--dataset"):
                    if flag in command:
                        path = Path(kwargs["cwd"]) / command[command.index(flag)+1]
                        rows = [json.loads(line) for line in path.read_text().splitlines()]
                        self.assertEqual([row["id"] for row in rows], ["positive", "negative"])
                        self.assertNotEqual(rows[0]["state"], rows[1]["state"])
                        self.assertTrue(all("images" not in row and "videos" not in row for row in rows))
                        if flag == "--dataset":
                            self.assertEqual([row["labels"]["visible"] for row in rows], [True, False])
                        checked.append(flag)
                return self.harness.bounded_run([sys.executable, str(stub), *command[1:]], **kwargs)
            def mcp(command, env, cwd, timeout):
                return self.harness.MCP([sys.executable, str(stub), *command[1:]], env, cwd, timeout)
            result = self.harness.run_matrix({"provider":"llamacpp", "model":"clef-flash",
                "jev":"/stub", "endpoint":"http://127.0.0.1:8080", "max_requests":21,
                "report":Path(temporary)/"report.json"}, runner=runner, mcp_factory=mcp)
            self.assertTrue(result["passed"], result["cases"])
            self.assertEqual(result["planned_requests"], 21)
            self.assertEqual(result["reserved_requests"], 21)
            self.assertEqual(checked.count("--input"), 3)
            self.assertEqual(checked.count("--dataset"), 1)
            self.assertEqual({item["case"] for item in result["skipped"]},
                             {"jpeg", "webp", "ordered-video", "images-and-image-eval"})
            self.assertIn("quality_evaluation", result)
            self.assertEqual(result["quality_evaluation"]["input_kind"], "text")
            self.assertEqual(result["quality_evaluation"]["evaluation_task"], "literal_text_reading")
            self.assertIn("do not measure vision", result["quality_evaluation"]["limitation"])

    def test_full_offline_matrix_checks_cli_files_and_all_mcp_tools(self):
        with tempfile.TemporaryDirectory() as temporary:
            stub = Path(temporary) / "stub.py"
            stub.write_text(STUB, encoding="utf-8")
            report = Path(temporary) / "report.json"
            def runner(command, **kwargs):
                return self.harness.bounded_run([sys.executable, str(stub), *command[1:]], **kwargs)
            def mcp(command, env, cwd, timeout):
                return self.harness.MCP([sys.executable, str(stub), *command[1:]], env, cwd, timeout)
            result = self.harness.run_matrix({"provider":"huggingface", "model":"clef-flash",
                "jev":"/stub", "endpoint":"http://127.0.0.1:8787", "max_requests":64,
                "report":report}, runner=runner, mcp_factory=mcp)
            self.assertTrue(result["passed"], result["cases"])
            self.assertEqual({item["case"] for item in result["cases"] if item["case"].startswith("mcp-")},
                {"mcp-noul","mcp-choice","mcp-score","mcp-ask","mcp-map"})
            self.assertTrue(any(item["case"] == "ordered-video" for item in result["cases"]))
            self.assertTrue(any(item["case"] == "sampled-video" for item in result["cases"]))
            self.assertTrue(any(item["case"] == "state-token-budget" for item in result["cases"]))
            self.assertTrue(any(item["case"] == "video-source-metadata" for item in result["cases"]))
            self.assertIn("quality_evaluation", result, "the smoke receipt must separate quality from integration")
            self.assertEqual(result["quality_evaluation"]["rows"], 2)
            self.assertEqual(len(result["quality_observations"]), 2)
            self.assertEqual(json.loads(report.read_text())["schema"], "jev.clef.smoke/v1")
            self.assertTrue(result.get("mcp", {}).get("clean_exit"), "MCP EOF must complete cleanly")

    def test_malformed_mcp_containers_preserve_failure_provenance(self):
        mutations = ["result['content']=[None]", "result['content']=None",
                     "result['tools']=[None]", "result['tools']=None"]
        for mutation in mutations:
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                stub = root / "stub.py"
                trigger = "tools/list" if "tools" in mutation else "tools/call"
                source = STUB.replace("  print(json.dumps({'jsonrpc'",
                    "  if method==" + repr(trigger) + ": " + mutation + "\n  print(json.dumps({'jsonrpc'")
                self.assertNotEqual(source, STUB)
                stub.write_text(source, encoding="utf-8")
                def runner(command, **kwargs):
                    return self.harness.bounded_run([sys.executable, str(stub), *command[1:]], **kwargs)
                def mcp(command, env, cwd, timeout):
                    return self.harness.MCP([sys.executable, str(stub), *command[1:]], env, cwd, timeout)
                report = root / "report.json"
                result = self.harness.run_matrix({"provider": "llamacpp", "model": "clef-flash",
                    "jev": "/stub", "endpoint": "http://127.0.0.1:8787", "max_requests": 64,
                    "report": report}, runner=runner, mcp_factory=mcp)
                self.assertFalse(result["passed"])
                self.assertIn("provenance", json.loads(report.read_text()))

    def test_json_output_refuses_symlink_parents_and_preserves_existing_file(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            directory = root / "directory"
            directory.mkdir()
            alias = root / "alias"
            alias.symlink_to(directory, target_is_directory=True)
            with self.assertRaises((OSError, ValueError)):
                self.harness.write_json(alias / "receipt.json", {"safe": True})
            self.assertFalse((directory / "receipt.json").exists())
            target = directory / "receipt.json"
            target.write_text("original")
            with self.assertRaises(FileExistsError):
                self.harness.write_json(target, {"safe": True})
            self.assertEqual(target.read_text(), "original")


# A controlled executable boundary; it performs no HTTP and never imports model code.
STUB = r'''
import json,sys,os
from pathlib import Path
a=sys.argv[1:]
assert a[a.index('--retries')+1]=='0'
assert not any(name.startswith(('TYPESAFE_','CLOUDFLARE_')) for name in os.environ)
def answer(q):
 kind=q['type']
 if kind=='noul': return {'type':kind,'noul':0.9}
 if kind=='choice':
  keys=list(q['criteria']); return {'type':kind,'choice':keys[0],'confidence':0.8,'probabilities':{keys[0]:0.8,keys[1]:0.2}}
 return {'type':kind,'score':0.8,'confidence':0.8,'probabilities':{'0':0.2,'1':0.8},'legend':{'0':q['criteria'][0],'1':q['criteria'][1]}}
def evaluation(q): return {'schema':'jev.evaluation/v1','answers':{k:answer(v) for k,v in q.items()}}
def rows(q):
 return [{'schema':'jev.map.row/v1','ok':True,'index':i,'id':name,'attempts':1,**{'answers':evaluation(q)['answers']}} for i,name in enumerate(['positive','negative'])]
def summary(resumed=0): return {'schema':'jev.map.summary/v1','total':2,'evaluated':2-resumed,'resumed':resumed,'succeeded':2-resumed,'failed':0,'complete':2,'stopped_early':False,'interrupted':False}
def mcp_questions(params):
 return {item['id']:{'type':item['type'],**({'criteria':{o['name']:None for o in item['options']}} if item['type']=='choice' else {'criteria':item['levels']} if item['type']=='score' else {})} for item in params['questions']}
if 'mcp' in a:
 for line in sys.stdin:
  call=json.loads(line)
  if 'id' not in call: continue
  method=call['method']
  if method=='initialize': result={'protocolVersion':'2025-06-18'}
  elif method=='tools/list': result={'tools':[{'name':name} for name in ['noul','choice','score','ask','map']]}
  else:
   tool=call['params']['name']; params=call['params']['arguments']
   if tool in ['map','ask']: q=mcp_questions(params)
   else:
    q={params['id']:{'type':tool,**({'criteria':{o['name']:None for o in params['options']}} if tool=='choice' else {'criteria':params['levels']} if tool=='score' else {})}}
   doc={'schema':'jev.mcp.map/v1','rows':rows(q),'summary':summary()} if tool=='map' else evaluation(q)
   result={'structuredContent':doc,'content':[{'type':'text','text':json.dumps(doc)}]}
  print(json.dumps({'jsonrpc':'2.0','id':call['id'],'result':result}),flush=True)
else:
 kind=next(name for name in ['noul','choice','score','ask','map','eval'] if name in a)
 if kind in ['ask','map','eval']: q=json.loads(Path(a[a.index('--request')+1]).read_text())['questions']
 else: q={'answer':{'type':kind,**({'criteria':{'red':None,'blue':None}} if kind=='choice' else {'criteria':['No red square','A red square is present']} if kind=='score' else {})}}
 if kind=='map':
  text=''.join(json.dumps(row)+'\n' for row in rows(q))
  if '--output-file' in a:
   file=Path(a[a.index('--output-file')+1])
   if '--resume' not in a: file.write_text(text)
  else: print(text,end='')
  print(json.dumps(summary(2 if '--resume' in a else 0)))
 elif kind=='eval':
  dataset=[json.loads(line) for line in Path(a[a.index('--dataset')+1]).read_text().splitlines()]
  sections={}
  for name,question in q.items():
   details=[]
   for row in dataset:
    label=row['labels'][name]
    if question['type']=='noul': predicted=0.9 if label else 0.1
    elif question['type']=='choice': predicted=label
    else: predicted=0.8 if label==1 else 0.2
    details.append({'id':row['id'],'label':label,'predicted':predicted,'correct':None if question['type']=='noul' else True})
   sections[name]={'type':question['type'],'n':len(details),'labelled':len(details),'rows':details}
  print(json.dumps({'schema':'jev.eval/v1','rows':{'total':len(dataset),'evaluated':len(dataset),'failed':0,'stopped_early':False,'interrupted':False},'questions':sections}))
 else: print(json.dumps(evaluation(q)))
'''


if __name__ == "__main__":
    unittest.main()

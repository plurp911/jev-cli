#!/usr/bin/env python3
"""Offline regressions for the explicit synthetic Clef quality benchmark."""
import importlib.util
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).with_name("clef-quality.py")


class QualityTests(unittest.TestCase):
    def setUp(self):
        self.assertTrue(SCRIPT.is_file(), "the reproducible quality benchmark is missing")
        spec = importlib.util.spec_from_file_location("clef_quality", SCRIPT)
        self.quality = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.quality)

    def test_original_cases_have_unique_images_and_explicit_labels(self):
        rows = self.quality.rows("cloudflare")
        self.assertEqual(rows, self.quality.rows("cloudflare"))
        self.assertEqual(len(rows), 32)
        self.assertEqual(len({row["id"] for row in rows}), 32)
        self.assertEqual(len({row["images"][0]["base64"] for row in rows}), 32)
        self.assertEqual(sum(row["labels"]["visible"] for row in rows), 8)
        text = self.quality.rows("llamacpp")
        self.assertTrue(all("images" not in row for row in text))
        self.assertEqual([row["labels"] for row in text], [row["labels"] for row in rows])

    def test_quality_failure_report_captures_before_after_cli_fingerprints(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            cli = root / "private-cli-path"
            cli.write_bytes(b"abc")
            config = self.config(root / "report.json")
            config["jev"] = cli
            def runner(command, **kwargs):
                self.assertEqual(command[0], str(cli))
                cli.unlink()
                return subprocess.CompletedProcess(command, 4, "private-canary", "private-canary")
            result = self.quality.run(config, runner=runner)
            receipt = result["provenance"]
            self.assertEqual(receipt["artifacts"][0]["before"]["sha256"], hashlib.sha256(b"abc").hexdigest())
            self.assertEqual(receipt["artifacts"][0]["status"], "missing")
            self.assertNotIn("private-canary", (root / "report.json").read_text())

    def test_quality_refuses_invalid_provenance_before_auth_and_runner(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest = root / "manifest"
            manifest.write_bytes(b"\xff")
            config = self.config(root / "report.json")
            config["provenance_manifest"] = manifest
            result = self.quality.run(config, runner=lambda *a, **kw: self.fail("must not run"))
            self.assertFalse(result["passed"])
            self.assertEqual(result["reserved_requests"], 0)
            self.assertEqual(result["provenance"]["status"], "refused")

    def test_quality_fixture_failure_writes_provenance_with_zero_calls(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            original = self.quality.smoke.write_json
            def fail_fixture(path, value):
                if Path(path).name == "questions.json":
                    raise OSError("private-canary")
                return original(path, value)
            with patch.object(self.quality.smoke, "write_json", side_effect=fail_fixture):
                result = self.quality.run(self.config(root / "report.json"),
                    runner=lambda *args, **kw: self.fail("fixture failure must not invoke CLI"))
            self.assertFalse(result["passed"])
            self.assertEqual(result["reserved_requests"], 0)
            self.assertIn("provenance", result)
            self.assertNotIn("private-canary", (root / "report.json").read_text())

    def test_disagreement_is_measured_without_becoming_an_integration_failure(self):
        rows = self.quality.rows("cloudflare")
        document = self.document(rows, probability=0.1)
        result = self.quality.measure(document, rows)
        self.assertEqual(result["agreement_at_0_5"], 0.75)
        self.assertAlmostEqual(result["brier_score"], 0.21)
        self.assertFalse(result["generalizable"])

    def test_partial_reordered_wrong_labels_or_nonfinite_predictions_are_refused(self):
        rows = self.quality.rows("cloudflare")
        for failure in ("partial", "reordered", "label", "nan", "stopped", "wrong-type"):
            document = self.document(rows, probability=0.8)
            observations = document["questions"]["visible"]["rows"]
            if failure == "partial":
                observations.pop()
            elif failure == "reordered":
                observations.reverse()
            elif failure == "label":
                observations[0]["label"] = not observations[0]["label"]
            elif failure == "nan":
                observations[0]["predicted"] = float("nan")
            elif failure == "wrong-type":
                document["questions"]["visible"]["type"] = "score"
            else:
                document["rows"]["stopped_early"] = True
            with self.subTest(failure=failure), self.assertRaises(self.quality.smoke.Failure):
                self.quality.measure(document, rows)

    def test_budget_refuses_before_first_process(self):
        with tempfile.TemporaryDirectory() as directory:
            config = self.config(Path(directory) / "report.json")
            config["max_requests"] = 31
            with self.assertRaises(self.quality.smoke.Failure):
                self.quality.run(config, runner=lambda *a, **kw: self.fail("must not run"))

    def test_malformed_nested_output_preserves_sanitized_failure_receipt(self):
        for field in ("document", "summary", "questions", "section", "observations", "observation", "bool-counter"):
            for wrong in (None, [], "private-canary"):
                with self.subTest(field=field, wrong=wrong), tempfile.TemporaryDirectory() as directory:
                    report = Path(directory) / "report.json"
                    rows = self.quality.rows("ollama")
                    value = self.document(rows, 0.1)
                    if field == "document":
                        value = wrong
                    elif field == "summary":
                        value["rows"] = wrong
                    elif field == "questions":
                        value["questions"] = wrong
                    elif field == "section":
                        value["questions"]["visible"] = wrong
                    elif field == "observations":
                        value["questions"]["visible"]["rows"] = wrong
                    elif field == "observation":
                        value["questions"]["visible"]["rows"][0] = wrong
                    else:
                        value["rows"]["failed"] = False
                    result = self.quality.run(self.config(report), runner=lambda command, **kw:
                        subprocess.CompletedProcess(command, 0, json.dumps(value), ""))
                    self.assertFalse(result["passed"])
                    self.assertIn("provenance", json.loads(report.read_text()))
                    self.assertNotIn("private-canary", report.read_text())

    def test_report_parent_replacement_cannot_redirect_write(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            reports, outside = root / "reports", root / "other"
            reports.mkdir()
            outside.mkdir()
            def runner(command, **kwargs):
                reports.rename(root / "original-reports")
                reports.symlink_to(outside, target_is_directory=True)
                return subprocess.CompletedProcess(command, 4, "", "")
            with self.assertRaises((OSError, ValueError)):
                self.quality.run(self.config(reports / "receipt.json"), runner=runner)
            self.assertFalse((outside / "receipt.json").exists())

    def test_failure_report_never_copies_untrusted_cli_diagnostics(self):
        with tempfile.TemporaryDirectory() as directory:
            report = Path(directory) / "report.json"
            result = self.quality.run(self.config(report), runner=lambda command, **kw:
                subprocess.CompletedProcess(command, 4, "private-canary", "Bearer private-canary"))
            self.assertFalse(result["passed"])
            self.assertNotIn("private-canary", report.read_text())

    def test_text_report_explicitly_distinguishes_literal_reading_from_vision(self):
        with tempfile.TemporaryDirectory() as directory:
            config = self.config(Path(directory) / "report.json")
            config["provider"] = "llamacpp"
            def runner(command, **kwargs):
                source = Path(kwargs["cwd"]) / command[command.index("--dataset") + 1]
                rows = [json.loads(line) for line in source.read_text().splitlines()]
                self.assertTrue(all("images" not in row for row in rows))
                return subprocess.CompletedProcess(command, 0, json.dumps(self.document(rows, 0.1)), "")
            result = self.quality.run(config, runner=runner)
            self.assertEqual(result["evaluation_task"], "literal_text_reading")
            self.assertFalse(result["comparable_across_input_kinds"])
            self.assertIn("states the color and shape", result["benchmark_limitation"])
            self.assertEqual(result["quality"]["input_kind"], "text")
            self.assertEqual(result["quality"]["evaluation_task"], "literal_text_reading")
            self.assertIn("do not measure vision", result["quality"]["limitation"])

    def test_complete_run_checks_named_fixture_and_preserves_measurements(self):
        with tempfile.TemporaryDirectory() as directory:
            report = Path(directory) / "report.json"
            def runner(command, **kwargs):
                self.assertEqual(command[command.index("--retries") + 1], "0")
                source = Path(kwargs["cwd"]) / command[command.index("--dataset") + 1]
                rows = [json.loads(line) for line in source.read_text().splitlines()]
                return subprocess.CompletedProcess(command, 0, json.dumps(self.document(rows, 0.1)), "")
            result = self.quality.run(self.config(report), runner=runner)
            self.assertTrue(result["passed"])
            self.assertEqual(result["reserved_requests"], 32)
            self.assertEqual(result["quality"]["agreement_at_0_5"], 0.75)
            self.assertEqual(result["quality"]["input_kind"], "image")
            self.assertEqual(result["quality"]["evaluation_task"], "synthetic_shape_vision")
            self.assertIn("synthetic image shape family", result["quality"]["limitation"])
            self.assertIn("no production calibration", result["quality"]["limitation"])
            self.assertFalse(result["quality"]["comparable_across_input_kinds"])
            self.assertNotIn("images", report.read_text())

    @staticmethod
    def config(report):
        return {"provider": "ollama", "model": "clef-flash", "jev": "/stub",
                "endpoint": "http://127.0.0.1:8787", "max_requests": 32,
                "timeout": 120, "report": report}

    @staticmethod
    def document(rows, probability):
        return {"schema": "jev.eval/v1", "rows": {"evaluated": 32, "failed": 0,
                "stopped_early": False, "interrupted": False}, "questions": {"visible": {
                "type": "noul", "n": 32, "rows": [{"id": row["id"], "label": row["labels"]["visible"],
                "predicted": probability} for row in rows]}}}


if __name__ == "__main__":
    unittest.main()

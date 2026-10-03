#!/usr/bin/env python3
"""Behavior checks for honest timing and memory measurements; no network required."""

from __future__ import annotations

import subprocess
import importlib.util
import io
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

RUNNER = Path(__file__).with_name("benchmark.py")
SPEC = importlib.util.spec_from_file_location("benchmark", RUNNER)
benchmark = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(benchmark)


class BenchmarkTests(unittest.TestCase):
    def invoke(self, *arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(RUNNER), *arguments],
            capture_output=True,
            text=True,
            check=False,
        )

    def test_success_runs_every_iteration_before_reporting(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            counter = Path(directory) / "calls"
            result = self.invoke(
                "time", "successful command", "3", sys.executable, "-c",
                "import sys; from pathlib import Path; "
                "p = Path(sys.argv[1]); "
                "p.write_text(p.read_text() + 'x' if p.exists() else 'x')",
                str(counter),
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(counter.read_text(), "xxx")
            self.assertIn("3 iterations", result.stdout)
            self.assertIn("ms/op", result.stdout)
            self.assertEqual(result.stderr, "")

    def test_failure_stops_iterations_and_reports_no_time_or_private_output(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            counter = Path(directory) / "calls"
            result = self.invoke(
                "time", "failing command", "3", sys.executable, "-c",
                "import sys; from pathlib import Path; "
                "p = Path(sys.argv[1]); "
                "p.write_text(p.read_text() + 'x' if p.exists() else 'x'); "
                "print('private child output'); sys.exit(7)", str(counter),
            )
            self.assertEqual(result.returncode, 1)
            self.assertEqual(counter.read_text(), "x")
            self.assertEqual(result.stdout, "")
            self.assertIn("command exited 7", result.stderr)
            self.assertNotIn("private child output", result.stderr)

    def test_missing_executable_reports_no_measurement(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            result = self.invoke("time", "missing command", "1", str(Path(directory) / "absent"))
            self.assertEqual(result.returncode, 1)
            self.assertEqual(result.stdout, "")
            self.assertIn("could not start", result.stderr)

    def test_invalid_iterations_are_refused_without_running(self) -> None:
        for iterations in ("0", "-1", "not-a-number"):
            with self.subTest(iterations=iterations):
                result = self.invoke("time", "bad count", iterations, sys.executable, "-c", "pass")
                self.assertEqual(result.returncode, 2)
                self.assertEqual(result.stdout, "")
                self.assertIn("positive integer", result.stderr)

    def test_failed_memory_probe_reports_no_memory(self) -> None:
        result = self.invoke("rss", "failing memory probe", sys.executable, "-c", "raise SystemExit(4)")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stdout, "")
        self.assertIn("command exited 4", result.stderr)

    def test_successful_memory_probe_reports_units_or_unavailability(self) -> None:
        result = self.invoke("rss", "memory probe", sys.executable, "-c", "pass")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue("KiB" in result.stdout or "unavailable on this platform" in result.stdout)

    def test_memory_units_normalize_mac_bytes_and_linux_kib(self) -> None:
        for platform, reported_rss in [("darwin", 42 * 1024 * 1024), ("linux", 42 * 1024)]:
            resource = SimpleNamespace(RUSAGE_CHILDREN=object(), getrusage=lambda _: SimpleNamespace(ru_maxrss=reported_rss))
            out = io.StringIO()
            with self.subTest(platform=platform), patch.object(benchmark, "run", return_value=True), \
                    patch.object(benchmark.sys, "platform", platform), patch.dict(sys.modules, {"resource": resource}), \
                    redirect_stdout(out):
                self.assertEqual(0, benchmark.main(["rss", "memory probe", "command"]))
            self.assertEqual(["43008", "KiB"], out.getvalue().split()[-2:])


if __name__ == "__main__":
    unittest.main()

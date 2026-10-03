#!/usr/bin/env python3
"""Prove the boundary check against real, isolated Cargo workspace manifests."""

from __future__ import annotations

import importlib.util
import io
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

SCRIPT = Path(__file__).with_name("check-architecture.py")
SPEC = importlib.util.spec_from_file_location("check_architecture", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise SystemExit("cannot load workspace architecture check")
architecture = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(architecture)


class ArchitectureTests(unittest.TestCase):
    def check(self, additions: dict[str, str]) -> tuple[int, str]:
        with tempfile.TemporaryDirectory(prefix="jev-architecture-") as directory:
            root = Path(directory)
            (root / "Cargo.toml").write_text(
                '[workspace]\nmembers = ["crates/*"]\nresolver = "2"\n', encoding="utf-8"
            )
            for name in architecture.ALLOWED:
                crate = root / "crates" / name
                (crate / "src").mkdir(parents=True)
                (crate / "src" / "lib.rs").write_text("", encoding="utf-8")
                (crate / "Cargo.toml").write_text(
                    f'[package]\nname = "{name}"\nversion = "0.0.0"\nedition = "2021"\n'
                    + additions.get(name, ""),
                    encoding="utf-8",
                )
            out, err = io.StringIO(), io.StringIO()
            with redirect_stdout(out), redirect_stderr(err):
                status = architecture.main(root)
            return status, out.getvalue() + err.getvalue()

    def test_the_existing_downward_graph_passes(self):
        status, output = self.check({
            "jev-cli": '[dependencies]\njev-client = { path = "../jev-client" }\njev-config = { path = "../jev-config" }\njev-core = { path = "../jev-core" }\n',
            "jev-client": '[dependencies]\njev-core = { path = "../jev-core" }\n',
        })
        self.assertEqual(status, 0, output)

    def test_client_cannot_depend_on_cli(self):
        status, output = self.check({"jev-client": '[dependencies]\njev-cli = { path = "../jev-cli" }\n'})
        self.assertEqual(status, 1)
        self.assertIn("jev-client -> jev-cli (normal)", output)

    def test_core_cannot_depend_on_configuration(self):
        status, output = self.check({"jev-core": '[dependencies]\njev-config = { path = "../jev-config" }\n'})
        self.assertEqual(status, 1)
        self.assertIn("jev-core -> jev-config (normal)", output)

    def test_configuration_cannot_depend_on_core_even_at_build_time(self):
        status, output = self.check({"jev-config": '[build-dependencies]\njev-core = { path = "../jev-core" }\n'})
        self.assertEqual(status, 1)
        self.assertIn("jev-config -> jev-core (build)", output)

    def test_target_specific_dev_edges_are_checked_on_other_hosts(self):
        status, output = self.check({"jev-core": '[target.\'cfg(windows)\'.dev-dependencies]\njev-client = { path = "../jev-client" }\n'})
        self.assertEqual(status, 1)
        self.assertIn("jev-core -> jev-client (dev, cfg(windows))", output)

    def test_dependency_renaming_does_not_hide_an_edge(self):
        status, output = self.check({"jev-client": '[dependencies]\nconfig = { package = "jev-config", path = "../jev-config" }\n'})
        self.assertEqual(status, 1)
        self.assertIn("jev-client -> jev-config (normal)", output)


if __name__ == "__main__":
    unittest.main()

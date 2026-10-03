#!/usr/bin/env python3
"""Measure successful subprocesses with a portable monotonic clock.

Usage: benchmark.py time LABEL ITERATIONS COMMAND [ARG...]
       benchmark.py rss LABEL COMMAND [ARG...]

Command streams and arguments are never printed: a failure reports only the label and
status, without producing a timing that could be mistaken for a successful operation.
"""

from __future__ import annotations

import subprocess
import sys
import time


def run(label: str, command: list[str]) -> bool:
    try:
        result = subprocess.run(
            command, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False
        )
    except OSError:
        print(f"benchmark {label!r}: could not start the command", file=sys.stderr)
        return False
    if result.returncode != 0:
        print(
            f"benchmark {label!r}: command exited {result.returncode}; no measurement reported",
            file=sys.stderr,
        )
        return False
    return True


def main(arguments: list[str]) -> int:
    if len(arguments) < 3 or arguments[0] not in {"time", "rss"}:
        print(__doc__, file=sys.stderr)
        return 2
    mode, label = arguments[:2]
    if mode == "time":
        try:
            iterations = int(arguments[2])
        except ValueError:
            iterations = 0
        if iterations < 1 or len(arguments) < 4:
            print("benchmark iterations must be a positive integer", file=sys.stderr)
            return 2
        started = time.perf_counter_ns()
        for _ in range(iterations):
            if not run(label, arguments[3:]):
                return 1
        milliseconds = (time.perf_counter_ns() - started) / 1_000_000
        print(
            f"  {label:<44} {milliseconds / iterations:8.2f} ms/op   "
            f"({iterations} iterations, {milliseconds:.2f} ms total)"
        )
    else:
        if not run(label, arguments[2:]):
            return 1
        try:
            import resource
        except ImportError:
            print(f"  {label:<44} unavailable on this platform")
            return 0
        # macOS reports bytes; Linux and the other supported Unix hosts report KiB.
        rss = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
        kibibytes = rss / 1024 if sys.platform == "darwin" else rss
        print(f"  {label:<44} {kibibytes:8.0f} KiB")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))

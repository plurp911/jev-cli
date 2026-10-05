# Benchmarks

## What is measured, and what is not

**Measured:** `jev`'s own overhead. How long the process takes to start, to resolve
configuration, to build a request, and to decode and render a response.

**Not measured:** Jev. Its accuracy, its calibration, and its latency belong to
TypeSafe, vary with the state and the question, and are not this tool's to benchmark.
A CLI benchmark that reported "Jev answered in 340 ms" would be reporting TypeSafe's
infrastructure and the reviewer's network, dressed up as a property of this tool.

Everything below runs against a local mock HTTP server on loopback. No API call is
made, no credential is needed, and nothing leaves the machine.

## Reproducing

```sh
scripts/bench.sh 300      # 300 iterations per measurement
```

The script builds the release binary, starts the mock, and prints the table. Fixtures
stay fixed at 200 and 2000 records; the argument controls process measurements. Use
`scripts/bench.sh 1` for a small process run with the full batch fixtures.

`scripts/benchmark.py` times successful subprocesses with Python's monotonic clock and
reports peak child memory in KiB on Linux and macOS. A failed command stops the run
without reporting a timing. Its own startup is outside the measured interval. Run
`python3 scripts/test-benchmark.py` to check those failure and success paths.
Memory includes the child's process launch, whose inherited memory can set a floor
above the executable's own needs. The `/bin/true` memory probe makes that floor visible;
figures close to it cannot distinguish the CLI's allocations.

The harness clears inherited credential sources, uses a dummy credential for its local
mock, disables the real OS keychain, and gives configuration and files an isolated
scratch directory. Cleanup stops and reaps its own server before removing that directory.
The current offline doctor measurement therefore excludes a real keychain round trip.

## Results

Recorded 2026-09-19 on Linux x86_64, with the calibration section added 2026-09-20, `rustc` 1.94.1, release profile (`lto = "thin"`,
`codegen-units = 1`, `panic = "abort"`, symbols stripped). **One machine, one run.**
Treat these as an order of magnitude, not a specification. These historical figures
predate both the MCP server in ADR-0012 and the Clef adapters and media support in
ADR-0015. They do not measure Clef model latency, vision/video processing, or the
separate Python bridge. The current binary includes its async dependencies,
with a runtime constructed only for `jev mcp serve`; rerun the harness for current
size, dependency counts, and timings.

### Process overhead

Each figure includes the cost of `fork`+`exec`, which is measured separately as a floor.

| Measurement | ms/op | Net of process spawn |
| --- | --- | --- |
| `/bin/true` (baseline) | 0.63 | — |
| `jev --version` | 1.13 | ~0.5 |
| `jev --help` | 1.19 | ~0.6 |
| `jev noul … --dry-run` | 1.18 | ~0.6 |
| `jev ask … --dry-run` (3 questions) | 1.23 | ~0.6 |
| `jev doctor --output json` | 7.48 | ~6.9 |

`doctor` is the outlier and legitimately so: it is the one command whose job is to probe
the OS credential store, which on Linux means a D-Bus round trip. Every other command
connects to the store only if it actually needs a credential.

### End to end, against the local mock

| Measurement | ms/op |
| --- | --- |
| `noul`, JSON output | 1.79 |
| `noul`, text output | 1.80 |
| `ask`, 3 questions, JSON output | 1.82 |
| `models` | 1.74 |

So roughly **0.6 ms of request/response handling** on top of startup, and JSON and text
rendering cost the same to within noise. Asking three questions instead of one costs
nothing measurable on the client — the batching win is entirely on the API side.

### Batch throughput

200 records, one process, one question set of three questions:

| Concurrency | ms for 200 records | ms/record |
| --- | --- | --- |
| `-j 1` | 31.2 | 0.16 |
| `-j 4` | 19.4 | 0.10 |
| `-j 16` | 26.8 | 0.13 |
| `-j 32` | 25.0 | 0.13 |

Against a mock that answers instantly, concurrency cannot help much and the numbers are
dominated by fixed cost — about **0.12 ms of client work per record**. Against the real
API, where a request takes far longer than 0.12 ms, concurrency is the whole game and
these figures say only that `jev` will not be the bottleneck. The spread across `-j`
values here is within run-to-run noise; do not read a best concurrency out of it.

### Where that per-record cost goes

Same 200 records at `-j 16`, varying one thing at a time:

| | ms for 200 records |
| --- | --- |
| JSONL in, stdout out | 21.2 |
| Plain lines in (`--lines`), stdout out | 21.0 |
| JSONL in, `--output-file` | 24.2 |

**JSONL parsing is not where the time goes.** Parsing a JSON record and looking up
`--state-field` costs about the same as not parsing at all — under 1% of the batch. The
measurable cost is `--output-file`, at roughly 3 ms per 200 records, which is the
deliberate per-row flush: rows are flushed as they complete so that a killed process
keeps them, and that trade is worth 0.015 ms a record.

### Scaling, and what `map` holds in memory

| Records at `-j 16` | ms | Peak RSS |
| --- | --- | --- |
| 200 | 23.2 | 10.0 MB |
| 2000 | 167.7 | 25.5 MB |

Ten times the records costs about seven times the wall clock — the difference is process
startup, which is fixed. **Memory grows with the input**, because records are read fully
before dispatch so that output can be written in input order and `--resume` can reason
about indexes (ADR-0010). It works out to roughly 9 KB per record here, on records of
about 80 bytes.

That is a real bound, and it is the reason `map` has one: the practical ceiling is
`--max-input-bytes` over the whole file, which defaults to 1 MiB and bites long before
the million-record cap. If you need more, raise it deliberately and watch the RSS, or
split the input — rows carry their own `index`, so the parts concatenate.

### Calibration, and what `jev eval` holds in memory

Recorded 2026-09-20, after `jev eval` landed. Sixty iterations for the small figures and
three to five for the batches, on the same machine and the same local mock.

| Measurement | ms/op |
| --- | --- |
| `eval --dry-run`, 200 rows (no network) | 2.20 |
| `eval`, 200 rows, no objective | 23.0 |
| `eval`, 200 rows, `maximize-f1` with a split | 23.6 |
| `eval`, 2000 rows, `maximize-f1` with a split | 168.7 |

**The local statistics cost essentially nothing.** Two hundred rows with no objective and
two hundred with a full sweep, a split, and a selection differ by 0.6 ms — inside the
noise — and 2000 rows costs about what `map` costs over the same 2000 records (168 ms
against 198 ms). The time is the requests, as it should be. The threshold sweep is linear
in the number of *distinct* observed values, and this confirms nothing quadratic has crept
into it.

`eval --dry-run` at 2.2 ms is the honest way to see what a large dataset would send
before sending it: it builds the same request bytes a real row would, for the first three
rows, and computes the split locally.

| Rows at `-j 16` | Peak RSS |
| --- | --- |
| `eval`, 2000 rows | 16.2 MB |
| `map`, 2000 records | 25.7 MB |

`eval` uses **less** memory than `map` over the same volume, which is worth stating
because the opposite would be the natural guess: `eval` has to hold every answer at once
for the metrics, where `map` can forget a row as soon as it is written. It comes out
ahead because it holds the decoded answers rather than the rendered JSON documents, and
because it never buffers rows for ordered output. Memory still grows with the dataset, for
the same reason `map`'s does, and the same `--max-input-bytes` ceiling applies —
`--limit` is the deliberate cap.

### Size and memory

| | |
| --- | --- |
| Release binary | 4.6 MB (4,764,536 bytes), stripped |
| Direct runtime dependencies | 8 |
| Crates in the runtime graph | 89 |
| Peak RSS, `ask` with 3 questions | 5.0 MB |
| Peak RSS, `map` over 200 records at `-j 16` | 10.0 MB |
| Peak RSS, `map` over 2000 records at `-j 16` | 25.7 MB |
| Peak RSS, `eval` over 2000 rows at `-j 16` | 16.2 MB |

The 89 crates are mostly TLS: `rustls`, `ring`, and the Mozilla root store, plus the
platform credential store. That historical build had no async runtime; the MCP server
added one subsequently (ADR-0012).

## Two bugs this benchmark found

Worth recording, because "we benchmarked it" is only useful if it changed something.

1. **The credential store was opened on every invocation.** `jev --version` connected to
   D-Bus before parsing its arguments, costing ~6 ms and doing something no user would
   expect from `--version`. The store is now lazy, connecting on the first `get`, `set`,
   or `delete`. Startup went from 6.8 ms to 1.1 ms.
2. **The HTTP connection pool was smaller than `map`'s concurrency.** `ureq` keeps three
   idle connections per host by default; above that, a batch reconnects — and against
   HTTPS, renegotiates TLS — on every record. The pool is now sized to the concurrency
   cap.

## Two measurement bugs, also worth recording

Both made the harness look like the tool, and both are the reason the mock server in
`scripts/bench.sh` has two unusual settings:

1. **Nagle on the mock** added about 40 ms to every keep-alive request, because Python's
   `BaseHTTPRequestHandler` writes headers and body separately. The first draft of this
   page would have reported 40 ms/record for `map`. It is 0.12 ms.
2. **The mock's listen backlog** is 5 by default. Above eight concurrent connections it
   refused them, `jev` correctly retried with backoff, and `-j 16` appeared to take a
   full second per batch.

If you write your own load test against a local server, check both before believing the
numbers.

## Comparison with other implementations

Not done, and deliberately not estimated. A fair comparison would need every project
installed at a pinned version, driven against the same mock, on the same machine, with
the same number of iterations — and the runtime-dependent ones (Python, Node) would need
their interpreter startup separated from their own work to mean anything.

What can be said without measuring: `jev` and `model-clis/jev` are the only native
binaries in the surveyed landscape, so they are the only two without an interpreter to
start. `docs/research/jev-cli-landscape.md` has the full comparison of what each project
does; it makes no performance claims either.

## What these numbers do not tell you

- **Nothing about accuracy.** A fast wrong answer is worse than a slow right one.
- **Nothing about real latency.** Against the API, the network and the model dominate
  by two to three orders of magnitude.
- **Nothing about your machine.** One run, one host, no variance reported. If a number
  here matters to a decision you are making, run `scripts/bench.sh` yourself.

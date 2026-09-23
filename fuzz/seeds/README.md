# Seed corpus

Small, hand-written inputs that `scripts/fuzz-smoke.sh` copies into `fuzz/corpus/`
before every run. Unlike the working corpus, these are committed.

## Why they exist

A fuzzer starting from random bytes spends most of a short run discovering that the
input is JSON. Seeding it with real documents means a 30-second CI run starts from a
successful parse and spends its budget on the interesting edges instead.

They are also where an input that once found a bug lives, so it is replayed on every
run forever — the same role `proptest-regressions/` plays for the property tests.

## What is in here

| Target | Seeds |
| --- | --- |
| `api_response` | One of each primitive, an unrecognised answer type, a `FastAPI` validation-error body, a model list, and the Score whose distribution mentions a level its legend does not. |
| `request_document` | Each primitive, a Choice with a `null` option description, a mixed set, and structured instructions. |
| `gate_expression` | A numeric comparison, a string equality, `not` around a nested `or`, a long conjunction, and `!=`. |
| `endpoint_url` | The official host plainly and oddly spelled, loopback, an IPv6 literal, a lookalike host, userinfo, and the octal-ambiguous host below. |
| `state_input` | Text, an object, an array, a truncated document, and a byte-order mark. |

## Findings kept here

* **`endpoint_url/octal-ambiguous-loopback.txt`** — `http://0000127.…077.2`. Read as
  loopback by the old `is_loopback`, resolved to the public address `87.10.63.2` by
  `getaddrinfo`, which treats a leading zero as octal. A credential would have crossed
  the internet in cleartext. See `docs/threat-model.md` T4.
* **`api_response/score-levels-from-distribution.json`** — a Score whose levels come
  partly from its distribution. The decoder was right and the fuzz target's invariant
  was the stale copy; both now derive the range the same way.

## Adding one

Keep it small and keep it meaningful — a seed is read by people as well as by the
fuzzer. A crash artifact becomes a seed *and* a unit test next to the parser that
produced it; neither substitutes for the other.

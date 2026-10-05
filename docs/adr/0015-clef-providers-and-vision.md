# ADR-0015: Explicit Clef providers and bounded vision inputs

* Status: Accepted
* Date: 2026-10-03

## Context

The maintainer requested hosted and open-source/local Clef and Clef Flash support,
including vision, and subsequently explicitly approved a publisher Python bridge
and video-frame support. The providers share the System One question/answer primitives,
but differ in routes, authentication, media representation, and limits. Treating a
Cloudflare base URL as a TypeSafe endpoint sends the wrong route and cannot decode
the response envelope. Treating all local servers identically sends the wrong media
format or offers capabilities their endpoints do not support.

## Options considered

1. Separate command families for every provider. This duplicates inference, gates,
   batches, evaluation, and MCP behavior.
2. Infer providers from model names or hosts. This makes a model flag change network
   destinations or credential lookup implicitly.
3. Explicit provider selection and adapters at the existing transport seam. Chosen.

## Decision

`--provider` selects TypeSafe, Cloudflare, Ollama, llama.cpp, or the local Python bridge. TypeSafe remains the
default. Validated `Endpoint` values carry their protocol, so existing clients and
batch workers route through the same request builder. Provider-specific envelopes
are decoded into the existing typed answers; returned confidence is preserved.

Credential trust remains separate from protocol: the official TypeSafe endpoint is
the only one that can access TypeSafe sources. Cloudflare and explicit remote
servers use the existing custom namespace. Explicit local protocols on loopback
never consult credentials and send no authorization header. Host overrides retain
the existing HTTPS/loopback and redirect rules. No automatic weight downloads or
server startup is introduced.

Vision consists of explicitly named image files or embedded images in named request
documents and selected JSONL fields. MCP accepts embedded images and prepared video frames and does not open
host paths. A validated core type enforces size, format-header, dimension, and
aggregate bounds before sending. It never decompresses image pixels. Data URLs and
MIME/base64 objects normalize to Cloudflare objects or Ollama raw base64 as required.
Template and row media sources cannot silently combine. Media and provider options
participate in fingerprints without changing unchanged TypeSafe resume behavior.

macOS's root `/tmp`, `/var`, and `/etc` aliases are normalized only after checking
their exact standard `/private` targets. The resulting path still undergoes the
same no-follow directory-handle walk; arbitrary symlinks remain refused.

Three direct runtime dependencies serve the input boundary:

| Dependency | Necessity, maintenance, and cost |
| --- | --- |
| `base64` 0.23, defaults disabled, `std` only | Bounded image serialization and decoding; already present transitively. Maintained public Rust implementation, MIT/Apache-2.0, no added transitive runtime dependencies. Reimplementing this hostile-input decoder adds unnecessary risk. |
| `imagesize` 0.15, defaults disabled, PNG/JPEG/WebP only | Reads dimensions without allocating a decoded pixel buffer. Maintained image-header parser, MIT, no transitive runtime dependencies or C toolchain. Multi-format hostile-header parsing is security-sensitive; our wrapper additionally validates complete signatures and bounds. |
| `rustix` 1.1, Unix only, defaults disabled, `std`/`fs` | Safe directory-relative no-follow, nonblocking image opens prevent path substitution and FIFO stalls without introducing unsafe code. Maintained MIT/Apache-2.0 implementation, already a runtime transitive dependency through clap; no new runtime dependency subtree. The standard library lacks these portable Unix open flags and directory-relative operations. |

The [provider guide](../clef.md) distinguishes upstream limits from the CLI's own
bounds and names unsupported features. Official sources and integration comparisons
are recorded in [research](../development/clef-research.md). No third-party code is
copied into the implementation.

## Consequences

All existing inference surfaces gain provider support through one seam. New flags,
configuration keys, media arguments, and provider metadata are additive CLI surfaces.
Existing defaults, exit-code meanings, and schema identifiers remain intact.
Local image bounds deliberately match hosted bounds even when a local server accepts
more. A compatible local server and weights are a user-managed prerequisite.

A separately launched Python bridge imports the publisher module from an explicitly
named existing release directory, with offline loader settings. That explicit act
executes trusted local model code; ordinary CLI invocations never start subprocesses.
Prepared frame arrays and constrained tokenization/processor controls use a documented
project-owned bridge contract. Request bodies, decoded pixels, and processor resize
budgets are bounded, and one request executes at a time. The bridge refuses browser
Origin headers and non-loopback Host headers. Its standard-library tests inject
inference and require no heavyweight model dependencies.

The 2026-10-04 publisher compatibility audit also establishes a wider JSON content
contract for this explicit Python provider: scalar and blank content, exact
instruction-ID fallback, explicit null Noul-side semantics, and dynamic Score
scales. Provider-aware parsing and a transport boundary check keep these values out
of narrower provider contracts. The Python path allows up to 255 Score levels as
an explicit client resource bound; TypeSafe/Cloudflare retain ten and Ollama 26.
The [separately reviewed compatibility decision](../development/clef-publisher-input-compatibility.md)
records primary evidence and the changed bridge expectation.

Hosted video, generation tools, training, telemetry, Gateway caching, and asynchronous
provider batch are not invented from marketing claims. Endpoint support follows
documented provider contracts.

## Revisit if

An upstream endpoint documents additional media types or model capabilities, or a
measured use case requires different local media bounds. Extend provider validation
with official evidence and regression tests; do not infer capabilities from a model
name or silently weaken input safeguards.

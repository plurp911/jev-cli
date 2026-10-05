# Troubleshooting

Start with `jev doctor`, using the same `--provider` and other global flags as the
failing command. It reports the provider, endpoint, model, configuration file,
which credential sources are populated, and where each setting came from — without
making a network request or printing your key.

```console
$ jev doctor
$ jev doctor --live      # queries the selected provider's model-listing endpoint
```

`--live` does not run inference. For example, `jev --provider ollama doctor --live`
checks a separately running Ollama server. A successful model listing does not prove
that its weights are loaded or that image/video inference works.

---

## "no TypeSafe API key found" (exit 3)

`jev` looked in `JEV_API_KEY`, `JEV_API_KEY_FILE`, `TYPESAFE_API_KEY`, and the OS
credential store, and found nothing.

```sh
jev auth login            # interactive, into the OS credential store
export JEV_API_KEY="…"    # CI and headless
```

`jev auth status` shows which sources are populated.

## "the API key found in … is empty" (exit 3)

A variable is set to whitespace. This is almost always a CI secret that did not get
substituted — a typo'd secret name, or a workflow where the secret is unavailable to a
fork. `jev` reports it rather than sending a blank key, because the resulting 401 is
much harder to diagnose.

## "the TypeSafe API rejected the credential (HTTP 401)" (exit 3)

The key reached the API and was refused:

```
error: the TypeSafe API rejected the credential (HTTP 401): Cannot authenticate with the server. Please check your API key and try again.
```

It is not retried. The key is revoked, mistyped, or not the one you think is in use —
`jev auth status` names the source that won, and a higher-precedence environment
variable is the usual surprise. `jev doctor --live` checks a key without spending model
tokens.

## "secure credential storage is unavailable" (exit 3)

There is no OS credential store `jev` can reach. On a headless Linux box that usually
means no running Secret Service, which is normal in a container or over SSH.

**`jev` will not fall back to a plaintext file.** Use the environment:

```sh
export JEV_API_KEY="…"
# or, with a secret manager that materializes to disk:
export JEV_API_KEY_FILE=/run/secrets/typesafe
```

## I ran `jev auth login` but it is still using a different key

An environment variable outranks the credential store, by design
([ADR-0008](adr/0008-credential-precedence-and-endpoint-isolation.md)). `jev auth login`
warns when it stores a key that something is shadowing. Check with:

```console
$ jev auth status
```

Unset the variable, or use it deliberately.

## "no API key found for the custom endpoint" (exit 3)

You passed `--endpoint`, or have one in your configuration file, and `jev` will not send
a TypeSafe credential to another host. Use the separate namespace:

```sh
export JEV_CUSTOM_API_KEY="…"
# or, with a secret manager:
export JEV_CUSTOM_API_KEY_FILE=/run/secrets/custom-provider
```

If you did not mean to use a custom endpoint, `jev config unset endpoint` or drop the
flag. `jev doctor` shows where the endpoint setting came from.

Cloudflare also uses only these custom key sources, even at its default endpoint.
Remote Ollama, llama.cpp, and Python-bridge endpoints use them too; loopback local
providers do not resolve or send credentials.

## Clef provider setup and media

### Cloudflare account or token is missing

Use `--provider cloudflare` explicitly and supply a 32-hexadecimal-character account
ID with `--cloudflare-account-id`, `CLOUDFLARE_ACCOUNT_ID`, or the saved
`cloudflare_account_id` setting. The account variable alone does not select
Cloudflare. Supply the Workers AI token through `JEV_CUSTOM_API_KEY` or
`JEV_CUSTOM_API_KEY_FILE`; `jev auth login` configures TypeSafe credentials only.

`jev doctor` reports an incomplete saved Cloudflare configuration without contacting
the server or looking up a credential. See [Cloudflare setup](clef.md#cloudflare).

### Cloudflare returns HTTP 422, code 5012 with `--reject-if-busy`

The retained hosted checks observed this error for Clef and Clef Flash on a Workers
AI Free account. They did not establish its precise cause or successful capacity
rejection. The CLI reports the error without silently removing the option. Omit
`--reject-if-busy` if you choose ordinary hosted inference; see
[the hosted limitations](clef.md#cloudflare). Ordinary hosted image inference
passed the [later synthetic checks](development/clef-live-testing.md#recorded-results);
that evidence does not establish capacity rejection or production accuracy.

### A local provider refuses the connection or takes too long

Start the chosen Ollama, llama.cpp, or [Python bridge](clef.md#publisher-python-weights-and-video)
server separately and verify that the selected endpoint and model name match it.
`jev` never starts a server, installs its runtime, downloads weights, or switches to
a cloud provider on failure. Default loopback ports are 11434 for Ollama, 8080 for
llama.cpp, and 8787 for the bridge. A llama.cpp model alias must match the running
server; its batch and microbatch sizes must fit the full Clef prompt, as described
in [local setup](clef.md#local-open-source-inference).

For slow CPU bridge inference, choose a larger explicit deadline and disable
duplicate attempts, for example `--timeout 600 --retries 0`. A timeout stops the
client waiting; it does not cancel an inference already running in PyTorch. A later
request can wait behind that work.

### An image or video is rejected before inference

Images require Cloudflare, Ollama with vision weights, or the Python bridge;
TypeSafe and llama.cpp do not accept Clef image input. Pass explicitly named PNG,
JPEG, or WebP files with `--image`, together with a state source. Ollama needs
nonempty text state; Cloudflare accepts `--state ''` with validated images.

Prepared video frames require `--provider huggingface` and the running bridge.
Repeat `--video-frame` in playback order for 1–32 equal-dimension frames; prepare
them yourself rather than passing a video file. Image paths must not contain
symlinks, except for the documented macOS system-root aliases. Byte, pixel, format,
and processor limits apply before sending; see [vision bounds](clef.md#vision-inputs-and-bounds).

Use the same provider flags with `--dry-run` to validate and inspect the request
without reading a credential or making a network call. Its output includes any
embedded media and supplied state.

## "refusing to send a credential over plain HTTP" (exit 2)

`http://` is only accepted for unambiguous loopback addresses. `localhost.example.com`
and `127.0.0.1.example.com` resolve to somebody else's machine and are not loopback.

"Unambiguous" also rules out an IPv4 octet with a leading zero, such as
`http://0127.0.0.1`. A resolver reads a leading zero as an octal marker and `jev` does
not, so the two could disagree about where the request is going — and a disagreement
there sends a credential over cleartext to a host nobody chose. Write the address the
ordinary way: `http://127.0.0.1:8080`.

## "the TypeSafe API rejected the request as invalid" (exit 2)

The server validated your request and refused it. The message names the field:

```
error: the TypeSafe API rejected the request as invalid (HTTP 422): questions.urgency.criteria: Field required
```

(One line. It is shown unwrapped here because that is how it is emitted — `jev` never
inserts a newline into an error message, so a log grep matches it whole.)

Common causes: a state larger than the model's 64k-token budget (or 32k for the state
plus the longest question), a model identifier that does not exist, or a question shape
`jev` does not validate locally. The first two come back as HTTP 400 with
`max_tokens_exceeded` and `Unknown model: …` respectively.

`jev ask … --dry-run` shows the exact body that would be sent.

## "the TypeSafe API is rate limiting or overloaded" (exit 4)

`jev` already retried with backoff and honoured any `retry-after` hint. The published
limits are 250,000 tokens per second and 1,200 requests per minute, and the
documentation says they change without notice.

For `jev map`, lower `--concurrency`. For a single call, `--retries` and `--timeout`
raise the budget.

## "the request timed out" (exit 4)

The per-attempt deadline is 10 seconds by default. A large state with many questions can
exceed it, and even a one-line state has been observed to take up to 9 seconds to
answer. A timed-out attempt is retried, and the
server may already have billed it, so for a slow call raise the deadline rather than the
retries:

```sh
jev ask -r big.json --timeout 60
```

## "the API returned a response this version cannot decode" (exit 4)

`jev` received something it could not read as a valid System One response. Either the
API changed, or you are pointed at something that is not the API.

Check `jev doctor` for the selected provider and endpoint. If they are correct and this
persists, please [open an issue](https://github.com/plurp911/jev-cli/issues) with the exact
message — but **not** the response body, which may contain your data.

## "--require could not be evaluated" (exit 6)

Your expression names a question id or a field that is not in the response. The most
common causes:

| Expression | Problem |
| --- | --- |
| `answer.confidence > 0.8` on a Noul | A Noul has **no** confidence. Use `answer.noul`. |
| `urgent.noul > 0.9` with `--id` unset | The default id is `answer`, not the question text. |
| `team.score > 1` on a Choice | Wrong primitive. Use `team.choice` or `team.confidence`. |
| `team.probabilities.Billing` | Option names are case-sensitive and must match exactly. |

Exit `6` is deliberately not exit `1`: a broken gate is not a negative judgment.

## The answer is not what I expected

Check the selected model, question design, and your own labelled examples. For
TypeSafe's Jev, the
[official documentation](https://docs.typesafe.ai/concepts/how-to-build-with-system-one)
is the place to take it. Briefly:

- **Define the boundary.** "Is the candidate strong in Python?" is unanswerable until
  "strong" is defined. Put the definition in `criteria`.
- **A Noul near 0.5 is not "medium".** It means yes and no are similarly likely. If you
  want a degree, use a Score with described levels.
- **Low confidence on a Choice** usually means no option is a clear winner — sometimes
  because two of them overlap, sometimes because none of them fits. Add an `other`
  option and see where the mass goes.
- **Give every option**, not a shortlist. The model cannot pick one you did not offer.
- **Check the jagged edges.** <https://docs.typesafe.ai/model-jaggedness/jev-1.13>
  documents known limitations of the current model.

## My results changed and I did not change anything

For TypeSafe, `jev-latest` is a moving alias. Check what actually answered:

```console
$ jev ask -r q.json -o json | jq -r '.model, .model_requested'
```

If those differ, the alias moved. Pin the version you calibrated against:

```sh
jev config set model jev-1.13.0
```

For Clef, also retain the provider, model revision, runtime, and quantization used
for evaluation. Thresholds measured for another provider or model do not establish
quality for the selected configuration.

## `jev` hangs with no output

It is waiting on stdin. A command with no `--state` flag reads state from standard
input; if stdin is a terminal, `jev` says so instead of waiting, but inside a script
with an inherited pipe it will block on the writer. Pass `--state` explicitly, or close
stdin with `< /dev/null`.

## The output has strange `\u{…}` sequences in it

That is the terminal sanitizer. Text from the API or from a file that contains ANSI
escapes, bidirectional overrides, or zero-width characters is escaped before display, so
it cannot rewrite your terminal. Use `--output json` to see the original bytes, safely
escaped by JSON instead.

## Something else

`--verbose` adds the credential source, the request shape, the attempt count, and the
elapsed time to stderr. It never prints your key, your state, or the response body.

```console
$ jev noul "…" --state-file x.txt --verbose
```

## `jev eval`

### `schema` is missing, not `jev.eval.row/v1`

The file passed to `--dataset` is not a labelled dataset. The likeliest cause is pointing
it at a `jev map` output file, or at the records you fed to `map`. A labelled row looks
like this, and every line needs the `schema` field:

```json
{"schema":"jev.eval.row/v1","id":"1","state":"…","labels":{"urgent":true}}
```

### `labels` names question `x`, which is not in …

The label's key has to be a question id from the `--request` file. It is refused rather
than ignored on purpose: a silently dropped label would look scored and would not be,
which is the worst possible failure for a command whose output is a number you will
trust. Check the spelling against the request file's `questions` keys.

### `…` is not one of this question's options

A Choice label must equal one of the option names exactly. It is not case-folded, because
that is the same comparison `--require 'team.choice == billing'` makes — folding here and
not there would mean the dataset validated under one rule and the gate ran under another.
The message lists the declared options; copy the spelling from it.

### a score label must be a whole level index from 0 to N

A Score label is the level's **position**, counting from zero, not its description.
Matching ground truth against the prose you wrote to prompt a model would mean
fuzzy-matching free text to decide what the right answer was, which is exactly the guess
a calibration tool must not make. `"minor"`, `"moderate"`, `"severe"` are levels `0`,
`1`, and `2`.

### `--objective …` applies to no question in this request

Objectives are routed by question type, and the routing is not a convention:

- A **noul** is swept on its probability, which is a decision cut. Use `maximize-f1`,
  `min-precision`, or `min-recall`.
- A **choice** or a **score** is swept on its confidence, which is an abstention axis.
  Use `min-accuracy` or `target-coverage`.

A noul has no confidence — the API returns none and `jev` does not invent one — so a
coverage objective on a request of nothing but nouls has nothing to sweep. The error is
raised before anything is sent.

**This checks the request file, not your labels.** If the request declares a Choice and
your dataset only labels the Noul beside it, the objective *does* apply to a question in
the request, so the run proceeds — and that question's `threshold` comes back `null` with
`objective_applies` false. Read the per-question section, not just the exit code.

### duplicate row id `x` (line N, and again at line M)

The `id` is the holdout split key, so a repeated one would put the same example on both
sides of the split and quietly inflate the result. The same check runs across
`--calibration` and `--test`: a row id in both files is refused for the same reason. Ids
have to be unique across whatever `jev eval` reads in one run.

### Exit `1`, and "no threshold reaches the target"

Not a failure of the run: the data was measured, the report is on stdout, and no cut in
your calibration rows meets the floor you asked for. Either the floor is higher than this
question can deliver on this data, or there are too few rows for any cut to demonstrate
it. Lower `--target`, collect more examples, or improve the question — but do not read a
relaxed number out of the sweep and use it as though it met the floor.

It is the same code a failed `--require` gate returns, and it is useful for the same
reason: a CI job can ask "does this question still clear 95% precision?" and branch.

### The numbers look worse than they did without `--objective`

Expected, and the point. With an objective, the threshold is chosen on one part of the
dataset and reported on the *rest* — so what you see is performance on rows the choice
never saw. Without an objective there is no selection, so nothing is held back and every
row is scored. If the two differ a lot, the threshold was fitting noise, which is exactly
what the split exists to reveal.

### "only N labelled row(s) were scored"

A small sample. The 95% interval printed beside each headline number is how wide the
uncertainty is; on forty rows it is usually wide enough to swallow the difference between
two thresholds. Collect examples that look like the traffic you will actually see. `jev`
will still compute everything — it just will not let the number stand without the caveat.

### Every row failed

Check `rows.errors` in `--output json`, which groups the failures by kind with one
example message each, and the stderr line, which names the first one. A whole batch
failing the same way is usually the credential (exit `3`), the endpoint, or a response
shape this version cannot decode.

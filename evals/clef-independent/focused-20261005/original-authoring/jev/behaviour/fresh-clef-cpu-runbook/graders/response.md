---
type: llm
---

Judge the response against the user request. Equivalent CLI flag ordering, quoting, option spellings (-O or --option), and any valid finite several-minute timeout appropriate for CPU work are acceptable; do not require a magic 600 seconds. Require all of the following:

- One correct command uses the explicit huggingface provider, clef-flash model, local publisher bridge at http://127.0.0.1:8787, a valid choice with cancel/keep options, and the supplied synthetic text as explicit state. An omitted --endpoint is acceptable only if the response establishes that the selected provider resolves to the stated bridge address. A machine-output flag is optional.
- The command has --retries 0 and an explicit valid --timeout at most 3600 seconds that allows several minutes (for example 300, 600, 900, or 1800). It must not use infinity or pretend a deadline bounds total compute.
- The explanation explicitly identifies the timeout as a per-attempt HTTP/client deadline and says that timing out or disconnecting does not cancel an active PyTorch computation.
- The explanation explicitly says the bridge serializes requests, so a later request may wait behind the still-running computation. It tells the operator to manage the runtime/ongoing work before resubmitting, rather than presenting immediate retries as cancellation or cleanup.
- It explicitly says jev uses an already started runtime and does not launch, install/download for, or stop it. Starting/stopping and setup/model preparation remain operator responsibilities. A concise sentence can cover these facts.
- It respects command-only planning: no inference execution or machine inspection, no installation/download execution, no invented result, and no requests for secrets. It must not switch to a hosted provider or promise that CLI timeout kills the model process.

Pass only if both the command and the requested explicit lifecycle/deadline explanation are correct. Merely showing the right flags without addressing the user's operational questions fails.

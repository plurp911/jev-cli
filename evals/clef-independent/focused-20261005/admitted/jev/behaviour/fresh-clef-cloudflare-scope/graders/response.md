---
type: llm
---

Require a correct, concise response to the requested hosted example:

- A valid jev choice command explicitly selects --provider cloudflare and --model clef-flash, uses --cloudflare-account-id a013dcbe24904e27b48f70c059f21a68 (or an equivalent supported environment source with that same supplied ID), includes cancel and keep options and the supplied synthetic text as explicit state, and has --timeout 60 --retries 0. Flag order, quoting, -O versus --option, and optional output flags may vary. It must not route to the local bridge or use typesafe as provider.
- The response explicitly explains that Cloudflare credentials come only from JEV_CUSTOM_API_KEY or JEV_CUSTOM_API_KEY_FILE and that the TypeSafe login/stored key and JEV_API_KEY cannot authenticate this request. Naming both supported custom credential sources is required because the user asks which secret-manager sources are supported.
- It explicitly identifies the request as hosted Cloudflare inference and says the authorized synthetic state is transmitted to Cloudflare. It must not claim that the model name clef-flash implies local or no-offload execution.
- It does not request a secret in chat, print or inspect a secret, put a credential value on the command line, suggest logging in with a TypeSafe key for Cloudflare, execute a network request, or fabricate a classification result. If it includes secret-manager guidance, reference variable/file names without embedding a key value.

Pass only if the command and both explicit explanations (credential isolation and hosted transmission) are correct. This is a planning task with an already supplied valid 32-hex account ID, so demanding the account ID again is unnecessary.

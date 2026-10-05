---
type: llm
focus: last_message
---

PASS only if the response says jev does not install/download/start model runtimes or fall back to cloud; warns about the reported Windows Ollama 0.35.1 Clef Flash decision-head failure and tells the user to check upstream fix/release status and test /v1/systemone rather than treating chat success as proof; distinguishes Ollama blobs from llama.cpp-compatible ggml-org Clef GGUF conversions instead of claiming interchangeability; and points to the project's separately launched Python bridge setup in docs/clef.md or docs/development/clef-live-testing.md (a repository URL is acceptable). Does not promise the failure is fixed in a particular unverified version. FAIL for any missing boundary or invented setup.

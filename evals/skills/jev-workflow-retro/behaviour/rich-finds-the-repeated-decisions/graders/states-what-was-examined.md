---
type: llm
focus: last_message
---

The report states its scope. The fixture's summary shows three providers (Claude Code,
Codex, Gemini CLI), fifteen sessions over 2026-09-02 to 2026-09-16, and two problems: one
Claude Code session with two unparsable lines, and one Codex rollout too old to parse.

PASS if the report states the providers, the session count and the date range, **and**
mentions both unreadable items (or says that some input could not be read and names
them). A provider that could not be read must appear, not be quietly absent.
FAIL if the report analyses the data without saying what it covered, omits a provider,
or omits the unreadable input.

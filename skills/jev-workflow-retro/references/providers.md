# Where each agent keeps its sessions, and what is actually supported

`scripts/transcripts.py` owns the parsing. This file is the note beside it: what was
observed, where, and why a provider is or is not parsed. Read it when a provider turns
up empty, when a user asks about an agent that is not in the list, or before changing an
adapter.

**Everything here was checked against real files on a real machine in September 2026.**
A path taken from a blog post and never opened is how a skill acquires an adapter that
has never worked. Where a format could not be verified, that is said rather than papered
over.

None of these formats is a published interface. They are internal state that changes
between releases, without a changelog and without a deprecation. Treat every adapter as
best-effort and every empty result as possibly a format change rather than an absence of
work.

---

## Parsed

### `claude-code`

```
~/.claude/projects/<cwd-with-slashes-as-dashes>/<sessionId>.jsonl
~/.claude/projects/<…>/<sessionId>/subagents/agent-<agentId>.jsonl
```

One JSON record per line. The records that matter:

| Want | Read |
| --- | --- |
| a prompt the person typed | `type:"user"`, `message.content` a string or a `text` block |
| tool output fed back | `type:"user"`, first content block `type:"tool_result"` |
| assistant prose | `type:"assistant"`, a `text` block in `message.content` |
| a tool call | `type:"assistant"`, a `tool_use` block: `name`, `input` |
| a subagent spawn | a `tool_use` named `Agent` — **or `Task`**, which is what older sessions call it — with `input.subagent_type` |
| a skill invocation | a `tool_use` named `Skill`, with `input.skill` |
| model, timestamp, project | `message.model`, `timestamp`, `cwd` |
| usage | `message.usage` |

Six traps, each of which produces a wrong number rather than an error:

1. **Not every `user` record is a person.** `isMeta` marks hook output and injected
   banners. Newer builds also carry `origin.kind`: `human` is a prompt, and
   `task-notification`, `peer` and `coordinator` are not — on a real week they were 40%
   of what had been counted as prompts. A record with no `origin` predates the field and
   is treated as typed.
2. **One response is several lines.** An assistant turn is split across records sharing
   a `requestId`. Summing `usage.output_tokens` over lines overcounts severalfold; take
   the maximum per request.
3. **A resumed session replays its history** into the new file. Deduplicate on `uuid`.
4. **The directory name is a lossy encoding of the path** — a real hyphen in a path
   segment is indistinguishable from a separator. The project comes from the `cwd` field
   inside the records; the directory name is only a fallback.
5. **The first line is bookkeeping** — `queue-operation`, `last-prompt`, `mode` — with no
   `cwd` or `version`. The session is opened on the first `user` or `assistant` record.
6. **A slash command is a routing decision**, arriving as a `<command-name>` wrapper in
   a user turn. It is emitted as a `skill` event with `via: slash-command`, not a prompt.
   Text the person pasted, wrapped in `<pasted_content>`, is still their prompt and is
   unwrapped rather than dropped.

**Subagent transcripts are separate files, one directory deeper**, at
`<project>/<sessionId>/subagents/agent-<agentId>.jsonl`, every record carrying
`isSidechain: true` and linked to the parent by the spawning `tool_use.id` → the tool
result's `toolUseResult.agentId`. A sibling `.meta.json` names the `agentType`.

They have to be globbed at both depths. A subagent's own turns exist *only* in its own
file — the parent records the spawn and the result, never what the subagent did — so a
one-level glob leaves the delegated half of every session unread. Their events roll into
the **parent** session's totals, because a subagent is not a session anybody started.

### `codex`

```
~/.codex/sessions/YYYY/MM/DD/rollout-<timestamp>-<uuid>.jsonl
```

Each line is `{timestamp, type, payload}`. The discriminator is the **pair**
`(type, payload.type)`, because `response_item` and `event_msg` each carry several
shapes. Checked against 12,082 rollouts, `cli_version` 0.101 to 0.155.1:

| Want | `type` | `payload.type` | Read |
| --- | --- | --- | --- |
| session, project | `session_meta` | — | the **first** one only: `payload.cwd`, `payload.id`, `payload.cli_version` |
| model, per turn | `turn_context` | — | `payload.model`, `payload.cwd`; also a turn boundary |
| prompt and assistant prose | `response_item` | `message` | `payload.role`, the `text` of each `payload.content` block |
| … the same turns again | `event_msg` | `user_message`, `agent_message` | `payload.message` |
| subagent spawn | `response_item` | `function_call` with `namespace: collaboration`, `name: spawn_agent` | `arguments.task_name` |
| … the same spawn again (0.147+) | `event_msg` | `item_completed`, `item.type: SubAgentActivity`, `kind: started` | `item.agent_path` |
| … and on 0.144–0.150 | `event_msg` | `sub_agent_activity` | `payload.agent_path`, `payload.kind` |
| MCP tool call (0.149+) | `event_msg` | `item_completed`, `item.type: McpToolCall` | `item.server`, `item.tool`, `item.status` |
| MCP tool call (0.147–0.148) | `event_msg` | `mcp_tool_call_end` | `payload.invocation.server`, `.tool` |
| compaction | `compacted`, or `event_msg`/`context_compacted` | — | marks the session |
| usage | `event_msg` | `token_count` | `payload.info.total_token_usage` — **the last one** |
| tool call | `response_item` | `function_call`, `custom_tool_call` | `payload.namespace` + `payload.name`, `payload.arguments` |
| tool output | `response_item` | `function_call_output`, `custom_tool_call_output` | `payload.call_id`, `payload.output` |

Seven traps, every one of which produced a wrong number without raising:

1. **`token_count` is a running total**, emitted after every turn — up to 1,688 times in
   one real file. Summing them overcounted usage about 26×. Only the last one is the
   session's usage.
2. **Every turn is recorded twice**, as `response_item/message` *and* as
   `event_msg/user_message` or `agent_message`, in every version on disk. (An earlier
   version of these notes said current builds had dropped the `event_msg` pair; they have
   not.) The adapter emits a turn's text once per turn, reset at each `turn_context`, so a
   person who genuinely types the same thing in a later turn is still counted.
3. **Injected user turns.** `<environment_context>`, `<subagent_notification>`,
   `<recommended_plugins>` and `<turn_aborted>` arrive as `role: user`. On real data they
   were 37% of what had been counted as prompts. A user turn made entirely of wrapper
   elements is dropped; the text around a wrapper is kept.
4. **One spawn, two records.** `collaboration.spawn_agent` and a `SubAgentActivity` with
   `kind: started` appear one for one. The call is the decision and is what counts; a
   "started" record counts only when no call preceded it. There is no agent *type* in
   either — only a task name and a path — so Codex subagents report `unspecified`.
5. **A subagent's rollout is a separate file** whose header names its parent
   (`source.subagent.thread_spawn.parent_thread_id`). It carries the parent's session id,
   like a Claude Code sidechain file, and its `user` turns — written by the parent agent,
   not the person — are not prompts. Two or three sibling subagents spawned together
   otherwise looked like the person typing the same instruction three times.
6. **A fork replays its parent's header on line 2.** Taking the last `session_meta` moved
   every event under the parent's id and left a one-event phantom session.
7. **MCP calls have no `function_call`.** They exist only as the `item_completed` or
   `mcp_tool_call_end` records above. Without those, every MCP tool the agent used was
   invisible — 750 in a one-in-ten sample of one store.

Every other `item_completed` item type — `AgentMessage`, `UserMessage`, `Reasoning`,
`CommandExecution`, `FileChange` — restates a record already handled and is ignored so it
is not counted twice. `response_item/reasoning` is not read, per the policy in `SKILL.md`.

`arguments` is JSON **inside a string**. `output` is free text with a build-specific
header, not JSON — treat it as opaque and take its length, not its meaning. Shell use on
0.144+ is almost entirely `custom_tool_call` named `exec`, so a tool histogram collapses
to that one name; the arguments say what was run.

The project lives inside the file, not in the path, so a `--project` filter has to read
the first line of each rollout. Skipping that step once made the filter drop every Codex
session while the report still said Codex had been examined.

Sessions written before the `{type, payload}` envelope existed yield nothing. That is the
intended degradation, and the file is reported as "no `{type, payload}` envelope" rather
than passing silently — a file that yields no events *and* no problem is
indistinguishable from a session in which nothing happened.

### `gemini-cli`

```
~/.gemini/tmp/<project>/chats/session-<timestamp>-<hash>.jsonl   JSON Lines
~/.gemini/tmp/<project>/chats/session-<timestamp>-<hash>.json    one whole document
```

**Two formats share the directory**, and which one a session is in is not a choice the
reader gets to make. The `.jsonl` files are JSON Lines. The `.json` files are a single
pretty-printed document — `{sessionId, projectHash, startTime, lastUpdated, kind,
messages, summary}` — and they tend to be the *largest* sessions. Reading one of them a
line at a time produces thousands of "unparsable line" problems and no events, while
discovery still counts it as a session; the adapter dispatches on the suffix.

In both, a `kind:"main"` header carries `sessionId`, `projectHash` and `startTime`, then
`type:"user"` and `type:"gemini"` records follow.

**A Gemini content part carries no `type` key.** It is `{"text": "…"}` — the native part
shape, not an Anthropic or OpenAI content block. Requiring `type == "text"` dropped every
prompt in the store while reporting nothing wrong. `displayContent` is usually `null`, and
where it exists it is a *list*, so it goes through the same joiner rather than straight
to the clipper.

The project is the directory name under `~/.gemini/tmp/` — a name, not a path — so the
same project audited through Gemini and through another provider counts as two in a
summary's `projects` total. Say so rather than merging on a basename, which would join
unrelated directories that happen to share one.

Lines containing `$set` are incremental updates to earlier records. Replaying them
double-counts turns; they are ignored.

### `cline`

```
~/.cline/data/sessions/<id>/<id>.messages.json     the turns
~/.cline/data/sessions/<id>/<id>.json              metadata, including cwd
```

A single JSON document with a `messages` array of `{role, content, ts}`, assistant turns
carrying `metrics` and `modelInfo`.

### `generic`

Any file the user names with `--input`:

- `.jsonl` — one record per line, roles read from `role` / `type` / `sender`;
- `.json` — a list, or an object with `messages` / `requests` / `conversation` /
  `turns` / `events` / `history`;
- `.md` / `.txt` — split on a line that is **nothing but** a speaker name
  (`## User`, `**Assistant**`, `Human:`). A sentence that merely begins "Assistant: …"
  is prose, and treating it as a boundary shreds a transcript into fragments that then
  get reported as frequency.

A record whose role cannot be established is emitted as `kind: "unknown"`, carrying its
role and size but never its text. That is deliberate twice over: a misattributed turn is
worse for a frequency count than an unattributed one, and an unknown the report can see
is better than a turn silently dropped — but a role nobody recognises may be a tool's
output, which is never reproduced. A role ending `_output`, `_result` or `_response` is
read as a tool result.

The default 30-day window does not apply to a file named with `--input`: the user chose
it, and it may be older. An explicit `--since`/`--until` does apply, and a record with no
timestamp under one is reported rather than silently kept. `--project` cannot apply to an
export and is reported as not applied.

---

### `cursor-agent`

```
~/.cursor/projects/<project>/agent-transcripts/<uuid>/<file>.jsonl
```

Cursor's terminal agent, not the editor. One `{role, message: {content: [...]}}` record
per line — the Anthropic message shape, with `text` and `tool_use` blocks — plus
occasional `{type, status}` records. Parsed by the generic record reader, which had been
reading `message` as a dict of keys and emitting the literal word "content" for every
turn of every one of these transcripts.

## Present on real machines, deliberately not parsed

Each of these has a real local store. None is parsed, and `discover` names them so that
a user is never left to infer that their history was not found.

| Provider | Store | Why not |
| --- | --- | --- |
| `cursor` (the editor) | `~/.config/Cursor/User/globalStorage/state.vscdb`, one per workspace, and `~/.cursor/chats/<hash>/<uuid>/store.db` | SQLite internals of a VS Code fork. No official export, no stability contract, and the community parsers disagree with each other. On the machine checked, `state.vscdb` held 12 `composerData` rows and 0 `bubbleId` rows, so the message shape could not be verified. Cursor's **agent CLI** is parsed; see above. |
| `copilot-chat` | `~/.config/Code/User/workspaceStorage/<hash>/chatSessions/<uuid>.json` | The files exist and the envelope is stable, but every session on the machine checked had `requests: []`, so the turn shape is unverified. An adapter written against a guess is worse than none. |
| `copilot-cli` | `~/.copilot/session-state/<uuid>/events.jsonl` | Documented, but not installed on the machine checked. It ships `/share file\|html\|gist`, which makes an export the better route. |
| `grok-cli` | `~/.grok/session_search.sqlite` | The only store on the machine checked, and its `session_docs` table was empty. |
| `antigravity-cli` | `~/.gemini/antigravity-cli/conversations/*.pb`, `*.db` | Protobuf and SQLite. Real, and Gemini CLI activity on the machine checked stopped when this began, so a Gemini user may find their recent history here rather than under `~/.gemini/tmp`. |
| `opencode` | `~/.local/share/opencode/opencode.db` | SQLite with JSON blobs in `message.data` and `part.data`, and gigabytes of it. |
| `pi` | `~/.pi/agent/sessions/<escaped-cwd>/<timestamp>_<uuid>.jsonl` | Documented and versioned, but not installed on the machine checked. Tree-structured with `parentId` branching, so it is not a flat replay. |
| `aider` | `.aider.chat.history.md` in the repository | Markdown, and the generic adapter already reads Markdown. Point `--input` at it. |

Also seen, unidentified or too thin to act on: `~/.agent/sessions/` (JSONL
`{kind, payload, ts}` with a `meta.json`), `~/.t3/userdata` (T3 Code provider logs),
`~/.continue/sessions`, and `~/.cline/data/db/sessions.db` beside the Cline files the
adapter does read.

## No local store at all

Do not look for a path for these; there is not one.

| Provider | Where the data is | Route |
| --- | --- | --- |
| hosted **x.ai / Grok Bot** conversations | xAI's servers. `~/.grokbot` exists on machines running the local-exec daemon, but holds its config, logs and an attachment staging area — no conversations. Do not mistake it for a store. | a manual account-data export from `accounts.x.ai/data`, then `--input` |
| **Amp** (Sourcegraph) | `ampcode.com/threads` | whatever export the product offers, then `--input` |

Saying "I could not find a local store for that agent, export a conversation and point
me at the file" is correct. Inventing `~/.grokbot/history.jsonl` is not: a path that does
not exist produces an empty result that reads exactly like "you have no opportunities
there".

---

## When an adapter stops working

The symptom is not an error. It is a provider that reports fewer sessions than the user
knows they have, or none.

1. `discover` first. If the store is there and sessions are zero, it is the date window
   or the format, not the path.
2. `events --provider <name> --limit 20`. Nothing, or only `session` records, means the
   record mapping moved.
3. Open one file and compare the record types against the table above.
4. Fix the mapping in `scripts/transcripts.py`, add the case to the jev-cli
   repository's own test suite, test-skill-scripts.py (it is not shipped with the
   skill), and update the row here.

**Say so in the report.** A provider whose adapter has drifted must appear in the
coverage statement as "examined, format not recognised", never be quietly absent.

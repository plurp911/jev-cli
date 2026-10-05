# Documentation

## Using `jev`

| Document | What is in it |
| --- | --- |
| [`install.md`](install.md) | Download, verify, and run a release archive on each supported platform. |
| [`commands.md`](commands.md) | Every command and flag, with examples. |
| [`clef.md`](clef.md) | Choose hosted Cloudflare or local Ollama, llama.cpp, and Python bridge inference; images, prepared video, and limits. |
| [`mcp.md`](mcp.md) | Agent-host setup, provider selection, tools, and media over MCP. |
| [`output-schema.md`](output-schema.md) | The JSON contract, document by document. |
| [`cli-contract.md`](cli-contract.md) | What scripts and agents can depend on: streams, exit codes, environment, configuration. |
| [`troubleshooting.md`](troubleshooting.md) | When something is not working. |
| [`agent-skill.md`](agent-skill.md) | The Agent Skills this repository ships, and how to install them. |
| [`release-verification.md`](release-verification.md) | Proving an artifact came from this repository. |

## Understanding it

| Document | What is in it |
| --- | --- |
| [`architecture.md`](architecture.md) | How the crates fit together, and why. |
| [`api-compatibility.md`](api-compatibility.md) | What `jev` assumes about the API, where each assumption comes from, and how it is checked. |
| [`threat-model.md`](threat-model.md) | What we defend against, and with what. |
| [`benchmarks.md`](benchmarks.md) | What was measured, what was not, and what the measuring found. |
| [`adr/`](adr/) | Architecture decision records. |
| [`repo-settings.md`](repo-settings.md) | GitHub settings for local verification and no automatic Actions. |
| [`development/skill-authoring.md`](development/skill-authoring.md) | How a shipped Agent Skill is written, evaluated, and called done. |
| [`development/agent-workflows.md`](development/agent-workflows.md) | Setup, read-only doctor, per-command proofs, exemplars, context sources, and drift maintenance. |
| [`development/clef-live-testing.md`](development/clef-live-testing.md) | Pinned local setup, opt-in synthetic tests, execution provenance, retained results, and limits of the evidence. |
| [`development/clef-documentation-skill-audit.md`](development/clef-documentation-skill-audit.md) | Documentation and skill corrections, fresh evaluation results, and retained evidence. |
| [`development/clef-research.md`](development/clef-research.md) | Dated primary-source research separating hosted schemas from local runtime/publisher capabilities. |
| [`development/clef-publisher-input-compatibility.md`](development/clef-publisher-input-compatibility.md) | Publisher JSON input compatibility and its scoped validation decision. |
| [`research/comparison.md`](research/comparison.md) | How `jev` compares with the other Jev CLIs, including where they are ahead. |
| [`research/`](research/) | The competitive landscape, and the provenance of the local reference corpus. |

For engineering rules, see [`AGENTS.md`](../AGENTS.md). For the TypeSafe API itself, see
the [official documentation](https://docs.typesafe.ai). Clef provider contracts come
from the [primary sources linked in the Clef guide](clef.md#capabilities-the-endpoints-do-not-expose);
the separately launched Python bridge has a project-owned contract in that guide.

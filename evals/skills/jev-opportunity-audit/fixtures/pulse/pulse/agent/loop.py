"""The assistant loop.

Agents can turn the assistant on for a conversation. It reads the thread, may
call any of the tools in ``pulse.agent.tools``, and finishes by writing an
internal note summarizing what it found. It never writes to the customer.
"""

from __future__ import annotations

import json
import logging
import os
from dataclasses import dataclass, field
from typing import Any, Awaitable, Callable

from anthropic import AsyncAnthropic

from pulse.agent.tools import TOOL_NAMES, TOOL_SCHEMAS

log = logging.getLogger(__name__)

_client = AsyncAnthropic(api_key=os.environ["ANTHROPIC_API_KEY"])

MODEL = "claude-sonnet-4-5"
ESCALATION_MODEL = "claude-haiku-4-5"

MAX_TURNS = 8

SYSTEM_PROMPT = """\
You are helping a support agent at Northwind Supply work through a customer
conversation. Gather the facts the agent needs using the tools available, then
write one internal note that lays out what you found and what is still
unknown. You are not talking to the customer and you cannot send email.
"""

_ESCALATION_PROMPT = """\
A support assistant is part-way through investigating a customer conversation.

What has happened so far:
{trace}

Should a human take over right now, before the assistant does anything else?
Answer with one word, yes or no.
"""

Handler = Callable[[dict[str, Any]], Awaitable[Any]]


@dataclass
class LoopState:
    conversation_id: str
    messages: list[dict[str, Any]] = field(default_factory=list)
    trace: list[str] = field(default_factory=list)
    escalated: bool = False


async def _should_escalate(state: LoopState) -> bool:
    """Ask whether this has stopped being something the assistant should do.

    Runs once per turn. The trace is short by construction, so this stays
    cheap even though it is on the hot path of every assistant conversation.
    """
    response = await _client.messages.create(
        model=ESCALATION_MODEL,
        max_tokens=5,
        temperature=0,
        messages=[
            {
                "role": "user",
                "content": _ESCALATION_PROMPT.format(
                    trace="\n".join(state.trace[-12:])
                ),
            }
        ],
    )

    answer = response.content[0].text.strip().lower()
    if answer.startswith("yes"):
        return True
    if answer.startswith("no"):
        return False

    log.warning("escalation check returned %r, treating as no", answer[:40])
    return False


async def run(
    state: LoopState,
    handlers: dict[str, Handler],
) -> LoopState:
    """Drive the assistant until it stops calling tools or runs out of turns."""
    missing = TOOL_NAMES - handlers.keys()
    if missing:
        raise RuntimeError(f"no handler registered for: {sorted(missing)}")

    for turn in range(MAX_TURNS):
        if await _should_escalate(state):
            state.escalated = True
            log.info("assistant handed %s back to a human", state.conversation_id)
            return state

        response = await _client.messages.create(
            model=MODEL,
            max_tokens=2000,
            system=SYSTEM_PROMPT,
            tools=TOOL_SCHEMAS,
            messages=state.messages,
        )

        state.messages.append({"role": "assistant", "content": response.content})

        tool_uses = [block for block in response.content if block.type == "tool_use"]
        if not tool_uses:
            return state

        results = []
        for block in tool_uses:
            state.trace.append(f"called {block.name} with {json.dumps(block.input)}")
            try:
                output = await handlers[block.name](block.input)
                results.append(
                    {
                        "type": "tool_result",
                        "tool_use_id": block.id,
                        "content": json.dumps(output, default=str),
                    }
                )
            except Exception as exc:  # surfaced to the model, not swallowed
                log.exception("tool %s failed on %s", block.name, state.conversation_id)
                state.trace.append(f"{block.name} failed: {exc}")
                results.append(
                    {
                        "type": "tool_result",
                        "tool_use_id": block.id,
                        "is_error": True,
                        "content": str(exc),
                    }
                )

        state.messages.append({"role": "user", "content": results})
        log.debug("turn %d used %d tools", turn, len(tool_uses))

    log.info("assistant hit the turn limit on %s", state.conversation_id)
    return state

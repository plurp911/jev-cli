"""Queue assignment for inbound conversations.

Every conversation that lands in the shared inbox passes through here exactly
once, on creation. The queue decides which group of agents sees the ticket in
their default view, so getting it wrong is visible: the customer waits behind
the wrong shift rotation.
"""

from __future__ import annotations

import json
import logging
import os
from dataclasses import dataclass

from openai import AsyncOpenAI

from pulse.intake import metrics

log = logging.getLogger(__name__)

QUEUES = ("billing", "shipping", "account", "other")

_PROMPT = """\
You are triaging a customer support email for an online retailer.

Subject: {subject}

Body:
{body}

Pick the team that should own this conversation.

- billing: invoices, charges, refunds, card declines, subscription price
- shipping: delivery, tracking, damaged or missing parcels, returns in transit
- account: sign-in, password, email changes, closing an account, data requests
- other: anything that fits none of the above

Think about it, then reply with JSON only, in this exact shape:
{{"queue": "billing"|"shipping"|"account"|"other"}}
"""

_client = AsyncOpenAI(api_key=os.environ["OPENAI_API_KEY"])


@dataclass(frozen=True)
class Conversation:
    id: str
    subject: str
    body: str


async def assign_queue(conversation: Conversation) -> str:
    """Return the queue name for ``conversation``.

    Called from the ingest worker for every inbound ticket before it becomes
    visible in the inbox. Falls back to "other" rather than raising: a ticket
    in the wrong queue is recoverable, a ticket that never appears is not.
    """
    prompt = _PROMPT.format(
        subject=conversation.subject,
        body=conversation.body[:4000],
    )

    response = await _client.chat.completions.create(
        model="gpt-4o-mini",
        messages=[{"role": "user", "content": prompt}],
        temperature=0,
        max_tokens=200,
        response_format={"type": "json_object"},
    )

    metrics.TICKETS_ROUTED.inc()
    raw = response.choices[0].message.content or "{}"

    try:
        payload = json.loads(raw)
    except json.JSONDecodeError:
        log.warning("router returned non-JSON for %s: %r", conversation.id, raw[:200])
        metrics.ROUTER_PARSE_FAILURES.inc()
        return "other"

    # The model is chatty about its reasoning even with response_format set,
    # and sometimes returns "confidence" or "why" alongside. We only want the
    # one field; everything else is dropped on the floor.
    queue = payload.get("queue")
    if queue not in QUEUES:
        log.warning("router returned unknown queue %r for %s", queue, conversation.id)
        metrics.ROUTER_PARSE_FAILURES.inc()
        return "other"

    log.info("routed conversation %s to %s", conversation.id, queue)
    return queue

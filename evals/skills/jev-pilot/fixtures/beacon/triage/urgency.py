"""Decide whether a ticket needs to be looked at today.

Calls the general-purpose chat model with a prompt that asks for one word. The caller
matches the first token and treats anything unexpected as "no", which has been the
source of two escalations.
"""

PROMPT = """You are a support triage assistant. Read the ticket and answer with
exactly one word, URGENT or ROUTINE. Do not explain.

Ticket:
{ticket}
"""


def is_urgent(client, ticket: str) -> bool:
    reply = client.complete(PROMPT.format(ticket=ticket), max_tokens=8)
    return reply.strip().upper().startswith("URGENT")

"""Synthetic ingest entry point for the intake paths described by this fixture."""

from pulse.intake.router import Conversation, assign_queue
from pulse.intake.urgency import score_urgency


async def ingest_ticket(conversation: Conversation) -> dict:
    """Compute the queue and inbox priority before making an inbound ticket visible."""
    queue = await assign_queue(conversation)
    urgency = score_urgency(conversation.subject, conversation.body)
    return {"id": conversation.id, "queue": queue, "urgency": urgency.score}

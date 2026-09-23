"""Relevance filtering over knowledge base candidates.

The vector query is generous and the composer is not: a draft that cites a
near-miss article reads worse than one that cites nothing. So every candidate
chunk is read by a large model and asked about directly.

Cost, because it comes up in every planning meeting: this file is the single
largest line on the OpenAI invoice. September was $4,180 of a $5,600 total,
and it scales with ticket volume rather than with headcount, so it grows when
the business does. Two attempts to move it to the mini model have been
reverted after the composer started citing the wrong return policy.
"""

from __future__ import annotations

import json
import logging
import os
from typing import TYPE_CHECKING, Sequence

from openai import AsyncOpenAI

if TYPE_CHECKING:
    from pulse.kb.search import Chunk

log = logging.getLogger(__name__)

_client = AsyncOpenAI(api_key=os.environ["OPENAI_API_KEY"])

MODEL = "gpt-4o"

# Chunks are up to 1200 characters, and we send them in batches so that one
# oversized article cannot push the batch past the context we are paying for.
BATCH_SIZE = 8
MAX_OUTPUT_TOKENS = 400

_PROMPT = """\
A support agent is answering this customer question:

{question}

Below are extracts from our help center. For each one, decide whether it
contains information that would actually help answer that question. Being
about the same broad topic is not enough.

{extracts}

Reply with JSON only: {{"relevant": ["<id>", ...]}}. Include only the ids of
the extracts that help. An empty list is a valid and often correct answer.
"""


def _render(chunks: Sequence["Chunk"]) -> str:
    return "\n\n".join(
        f"[{chunk.id}] {chunk.title}\n{chunk.body[:1200]}" for chunk in chunks
    )


async def keep_relevant(question: str, candidates: Sequence["Chunk"]) -> list[str]:
    """Return the ids of the candidates that survive the relevance pass.

    Batches are issued sequentially rather than concurrently. We hit the
    organization's tokens-per-minute ceiling during the Black Friday backlog
    and the retries cost more than the latency we saved.
    """
    kept: list[str] = []

    for start in range(0, len(candidates), BATCH_SIZE):
        batch = candidates[start : start + BATCH_SIZE]
        response = await _client.chat.completions.create(
            model=MODEL,
            messages=[
                {
                    "role": "user",
                    "content": _PROMPT.format(
                        question=question[:2000],
                        extracts=_render(batch),
                    ),
                }
            ],
            temperature=0,
            max_tokens=MAX_OUTPUT_TOKENS,
            response_format={"type": "json_object"},
        )

        raw = response.choices[0].message.content or "{}"
        try:
            relevant = json.loads(raw).get("relevant", [])
        except json.JSONDecodeError:
            # Dropping the batch is safer than keeping all of it. A thin set of
            # extracts produces a vaguer draft; a wrong extract produces a
            # confident wrong draft.
            log.warning("rerank returned non-JSON, dropping batch: %r", raw[:200])
            continue

        known = {chunk.id for chunk in batch}
        kept.extend(chunk_id for chunk_id in relevant if chunk_id in known)

    return kept

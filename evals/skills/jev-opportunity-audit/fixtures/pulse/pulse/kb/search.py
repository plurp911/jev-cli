"""Knowledge base retrieval.

Two stages. Stage one is a pgvector nearest-neighbor query over article
chunks, which is fast and indiscriminate. Stage two is
``pulse.kb.rerank.keep_relevant``, which is neither.
"""

from __future__ import annotations

import logging
from dataclasses import dataclass

from sqlalchemy import text
from sqlalchemy.ext.asyncio import AsyncSession

from pulse.kb.rerank import keep_relevant

log = logging.getLogger(__name__)

# Recall against our labeled set stops improving past about 40. We kept it at
# 40 rather than 20 because the second stage was supposed to absorb the noise.
CANDIDATE_LIMIT = 40

# What the composer and the assistant actually get handed.
RESULT_LIMIT = 6

_SEARCH_SQL = text(
    """
    SELECT c.id,
           c.article_id,
           a.title,
           c.body,
           1 - (c.embedding <=> :query_embedding) AS similarity
      FROM kb_chunk c
      JOIN kb_article a ON a.id = c.article_id
     WHERE a.published
       AND (a.audience = 'public' OR :include_internal)
     ORDER BY c.embedding <=> :query_embedding
     LIMIT :limit
    """
)


@dataclass(frozen=True)
class Chunk:
    id: str
    article_id: str
    title: str
    body: str
    similarity: float


async def search(
    session: AsyncSession,
    question: str,
    query_embedding: list[float],
    *,
    include_internal: bool = False,
) -> list[Chunk]:
    """Return the chunks worth showing for ``question``."""
    rows = await session.execute(
        _SEARCH_SQL,
        {
            "query_embedding": query_embedding,
            "include_internal": include_internal,
            "limit": CANDIDATE_LIMIT,
        },
    )

    candidates = [
        Chunk(
            id=row.id,
            article_id=row.article_id,
            title=row.title,
            body=row.body,
            similarity=float(row.similarity),
        )
        for row in rows
    ]

    if not candidates:
        return []

    kept_ids = await keep_relevant(question, candidates)
    by_id = {chunk.id: chunk for chunk in candidates}

    # keep_relevant returns ids only; anything it does not name is discarded,
    # including chunks with a high similarity score.
    surviving = [by_id[chunk_id] for chunk_id in kept_ids if chunk_id in by_id]
    log.info(
        "kb search kept %d of %d candidates for %r",
        len(surviving),
        len(candidates),
        question[:80],
    )
    return surviving[:RESULT_LIMIT]

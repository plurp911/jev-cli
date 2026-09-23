"""Urgency scoring for the inbox sort order.

The inbox sorts on ``urgency`` descending, then on age. This module produces
that number. It is deliberately cheap: it runs inline on the request path
during ingest, and it runs again whenever a customer replies to an open
conversation.
"""

from __future__ import annotations

import re
from dataclasses import dataclass

from pulse.intake import metrics

# Weighted phrases, highest first. Weights were picked by hand in 2023 by
# reading a few hundred tickets and arguing about them, and nobody has revised
# them since.
#
# maintainer note (dmitri, Apr): this list is wrong more often than I would
# like. "cancel my account" outranks a genuinely broken order because the word
# "cancel" is in here twice, and a customer who writes three calm paragraphs
# about a missing parcel scores zero. Do not add more phrases to fix a single
# ticket; that is how we got to 25.
#
# TODO: people reword things. "been waiting since Tuesday", "nobody has got
# back to me", "this is the third email" all mean the same as entries below
# and none of them match. Substring matching cannot close that gap.
URGENCY_PHRASES: list[tuple[str, int]] = [
    ("chargeback", 9),
    ("legal action", 9),
    ("solicitor", 8),
    ("small claims", 8),
    ("fraud", 8),
    ("unauthorized charge", 8),
    ("cancel my account", 7),
    ("cancel my subscription", 7),
    ("close my account", 7),
    ("third time", 6),
    ("fourth time", 6),
    ("still waiting", 6),
    ("no one has replied", 6),
    ("nobody replied", 6),
    ("unacceptable", 5),
    ("furious", 5),
    ("appalling", 5),
    ("disgusting", 5),
    ("complaint", 4),
    ("escalate", 4),
    ("manager", 4),
    ("urgent", 4),
    ("asap", 3),
    ("immediately", 3),
    ("right now", 2),
]

# Above this the ticket is shown with a red flag and counts toward the
# "needs attention" tile on the team dashboard.
URGENT_THRESHOLD = 11

_ALL_CAPS_RUN = re.compile(r"\b[A-Z]{4,}\b")
_EXCLAMATIONS = re.compile(r"!{2,}")


@dataclass(frozen=True)
class UrgencyResult:
    score: int
    is_urgent: bool
    matched: list[str]


def score_urgency(subject: str, body: str) -> UrgencyResult:
    """Score a message from 0 upward. Higher means sort it nearer the top."""
    haystack = f"{subject}\n{body}".lower()

    score = 0
    matched: list[str] = []
    for phrase, weight in URGENCY_PHRASES:
        if phrase in haystack:
            score += weight
            matched.append(phrase)

    # Shouting and repeated punctuation are weak signals on their own, so they
    # are capped well below any single phrase.
    score += min(len(_ALL_CAPS_RUN.findall(subject)) * 2, 4)
    score += min(len(_EXCLAMATIONS.findall(subject + body)), 3)

    metrics.URGENCY_SCORED.inc()
    return UrgencyResult(
        score=score,
        is_urgent=score >= URGENT_THRESHOLD,
        matched=matched,
    )

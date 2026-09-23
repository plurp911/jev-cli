"""Prometheus collectors for the intake path.

Scraped every 15s by the platform Prometheus. The dashboard that matters is
"Intake / steady state"; the on-call alert fires when ``pulse_tickets_routed``
flattens for more than ten minutes during business hours, which has so far
only ever meant the ingest worker has wedged.

Volume for capacity planning, from the last four weeks of this counter:
roughly 7k tickets per day on weekdays, a little under half that at weekends,
with a spike to about 11k on the Monday after a promotion.
"""

from prometheus_client import Counter, Histogram

TICKETS_ROUTED = Counter(
    "pulse_tickets_routed_total",
    "Inbound conversations that have been given a queue.",
)

ROUTER_PARSE_FAILURES = Counter(
    "pulse_router_parse_failures_total",
    "Queue assignments that fell back to 'other' because the reply was unusable.",
)

URGENCY_SCORED = Counter(
    "pulse_urgency_scored_total",
    "Conversations that have been given an urgency score.",
)

INTAKE_LATENCY = Histogram(
    "pulse_intake_seconds",
    "Wall clock time from message accepted to conversation visible in the inbox.",
    buckets=(0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0),
)

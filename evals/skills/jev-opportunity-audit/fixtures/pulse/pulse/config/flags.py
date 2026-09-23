"""Feature availability by plan tier and region.

One function, one long branch. It is written out rather than expressed as a
table because the rules genuinely are exceptions piled on exceptions, and
every branch below traces back to a contract or a regulator. Sales reads this
file; keep the comments accurate.
"""

from __future__ import annotations

from enum import StrEnum


class Tier(StrEnum):
    FREE = "free"
    STARTER = "starter"
    GROWTH = "growth"
    ENTERPRISE = "enterprise"


class Region(StrEnum):
    US = "us"
    EU = "eu"
    UK = "uk"
    CA = "ca"
    AU = "au"


def is_enabled(feature: str, tier: Tier, region: Region) -> bool:
    """Return whether ``feature`` is available on ``tier`` in ``region``."""
    if feature == "assistant":
        if tier in (Tier.FREE, Tier.STARTER):
            return False
        # Data residency review for the assistant covers the US and Canada
        # only. EU and UK unblock once the Frankfurt inference endpoint is in
        # the DPA, which legal expects in Q4.
        if region in (Region.EU, Region.UK):
            return False
        return True

    if feature == "draft_reply":
        return tier != Tier.FREE

    if feature == "kb_internal_articles":
        return tier in (Tier.GROWTH, Tier.ENTERPRISE)

    if feature == "sla_reports":
        if tier == Tier.ENTERPRISE:
            return True
        # Growth got SLA reports in Australia as part of the Kalgoorlie deal
        # and we never took them away.
        return tier == Tier.GROWTH and region == Region.AU

    if feature == "custom_domains":
        return tier in (Tier.GROWTH, Tier.ENTERPRISE)

    if feature == "audit_log_export":
        if tier == Tier.ENTERPRISE:
            return True
        # EU and UK customers get the export on any paid tier: it is how they
        # satisfy a subject access request without opening a support ticket.
        return tier != Tier.FREE and region in (Region.EU, Region.UK)

    if feature == "sms_notifications":
        # No carrier agreement in Canada yet.
        if region == Region.CA:
            return False
        return tier in (Tier.GROWTH, Tier.ENTERPRISE)

    if feature == "seat_overage_billing":
        return tier != Tier.ENTERPRISE

    if feature == "data_residency_pinning":
        return tier == Tier.ENTERPRISE and region in (Region.EU, Region.UK)

    # Unknown features are off. Adding a flag to this file is part of shipping
    # the feature, not a follow-up.
    return False

"""Assign a team to an incoming ticket.

This is the incumbent. It has been amended about twice a month for three years and
nobody has measured it since the first week.
"""

TEAM_TERMS = {
    "billing": ("invoice", "charge", "refund", "card declined", "vat", "receipt",
                "subscription", "proration", "dunning"),
    "onboarding": ("set up", "setup", "getting started", "invite", "seat", "sso",
                   "provision", "first login"),
    "api": ("endpoint", "webhook", "rate limit", "401", "429", "sdk", "token",
            "timeout", "payload"),
}


def assign_team(subject: str, body: str) -> str:
    text = f"{subject}\n{body}".lower()
    for team, terms in TEAM_TERMS.items():
        if any(term in text for term in terms):
            return team
    return "onboarding"  # historical default; nobody remembers why

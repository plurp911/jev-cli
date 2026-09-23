"""Money arithmetic and display.

All amounts move through the system as integer minor units (cents, pence)
paired with an ISO 4217 code. Floats are never used for money anywhere in
Pulse, and this module is the only place that turns an amount into a string.
"""

from __future__ import annotations

from dataclasses import dataclass
from decimal import ROUND_HALF_UP, Decimal

# Exponent per currency: how many minor units make one major unit. Only the
# currencies we actually charge in are listed; anything else raises.
_EXPONENTS = {
    "USD": 2,
    "EUR": 2,
    "GBP": 2,
    "CAD": 2,
    "AUD": 2,
    "JPY": 0,
    "KRW": 0,
}

_SYMBOLS = {
    "USD": "$",
    "EUR": "€",
    "GBP": "£",
    "CAD": "CA$",
    "AUD": "A$",
    "JPY": "¥",
    "KRW": "₩",
}

# Locales that put the symbol after the amount, with a non-breaking space.
_SUFFIX_LOCALES = frozenset({"de_DE", "fr_FR", "es_ES", "it_IT", "fi_FI"})

_GROUP_SEPARATORS = {
    "en_US": (",", "."),
    "en_GB": (",", "."),
    "en_CA": (",", "."),
    "en_AU": (",", "."),
    "de_DE": (".", ","),
    "es_ES": (".", ","),
    "it_IT": (".", ","),
    "fr_FR": (" ", ","),
    "fi_FI": (" ", ","),
}

NBSP = " "


class UnsupportedCurrency(ValueError):
    """Raised for a currency code this module has no rules for."""


@dataclass(frozen=True)
class Money:
    minor_units: int
    currency: str

    def __post_init__(self) -> None:
        if self.currency not in _EXPONENTS:
            raise UnsupportedCurrency(self.currency)

    def __add__(self, other: "Money") -> "Money":
        if other.currency != self.currency:
            raise ValueError(f"cannot add {other.currency} to {self.currency}")
        return Money(self.minor_units + other.minor_units, self.currency)

    def __sub__(self, other: "Money") -> "Money":
        return self + Money(-other.minor_units, other.currency)


def apply_rate(amount: Money, rate: Decimal) -> Money:
    """Multiply by a rate (tax, discount, proration) and round half up.

    Half up, not banker's rounding: it is what the invoicing system upstream
    does, and a one-cent disagreement on a line item fails reconciliation.
    """
    product = (Decimal(amount.minor_units) * rate).quantize(
        Decimal("1"), rounding=ROUND_HALF_UP
    )
    return Money(int(product), amount.currency)


def split_evenly(amount: Money, parts: int) -> list[Money]:
    """Split into ``parts``, distributing the remainder over the first parts."""
    if parts < 1:
        raise ValueError("parts must be at least 1")
    base, remainder = divmod(amount.minor_units, parts)
    return [
        Money(base + (1 if index < remainder else 0), amount.currency)
        for index in range(parts)
    ]


def format_money(amount: Money, locale: str = "en_US") -> str:
    """Render an amount for display. Never used for storage or comparison."""
    exponent = _EXPONENTS[amount.currency]
    group, decimal_point = _GROUP_SEPARATORS.get(locale, (",", "."))

    negative = amount.minor_units < 0
    digits = str(abs(amount.minor_units)).rjust(exponent + 1, "0")

    whole, fraction = (digits, "") if exponent == 0 else (
        digits[:-exponent],
        digits[-exponent:],
    )

    grouped = ""
    for offset, digit in enumerate(reversed(whole)):
        if offset and offset % 3 == 0:
            grouped = group + grouped
        grouped = digit + grouped

    body = grouped if not fraction else f"{grouped}{decimal_point}{fraction}"
    symbol = _SYMBOLS[amount.currency]

    if locale in _SUFFIX_LOCALES:
        rendered = f"{body}{NBSP}{symbol}"
    else:
        rendered = f"{symbol}{body}"

    return f"-{rendered}" if negative else rendered

"""RFC 2822 header parsing and reply threading.

Everything here is exact. Threading decides whether a customer's reply joins
the conversation they are replying to or opens a new one, and a wrong answer
splits a thread in a way support agents cannot repair from the UI.
"""

from __future__ import annotations

import re
from dataclasses import dataclass
from datetime import datetime, timezone
from email.header import decode_header, make_header
from email.utils import getaddresses, parsedate_to_datetime

# Message-IDs are angle-bracket delimited and may be separated by folding
# whitespace inside References. We are strict on purpose: a malformed ID is
# dropped rather than guessed at.
_MESSAGE_ID = re.compile(r"<([^<>@\s]+@[^<>@\s]+)>")

_UNFOLD = re.compile(r"\r?\n[ \t]+")


class HeaderError(ValueError):
    """Raised when a header is present but cannot be parsed."""


@dataclass(frozen=True)
class ParsedHeaders:
    message_id: str | None
    in_reply_to: str | None
    references: tuple[str, ...]
    sender: str | None
    recipients: tuple[str, ...]
    subject: str
    date: datetime | None


def unfold(value: str) -> str:
    """Collapse RFC 2822 folding whitespace into single spaces."""
    return _UNFOLD.sub(" ", value).strip()


def parse_message_id(value: str | None) -> str | None:
    """Return the single Message-ID in ``value``, without angle brackets."""
    if value is None:
        return None
    ids = _MESSAGE_ID.findall(unfold(value))
    if not ids:
        return None
    if len(ids) > 1:
        raise HeaderError(f"expected one Message-ID, found {len(ids)}")
    return ids[0]


def parse_references(value: str | None) -> tuple[str, ...]:
    """Return the Message-IDs in a References or In-Reply-To header, in order.

    Duplicates are removed but order is preserved: the last entry is the
    immediate parent, and the walk in ``thread_parent`` depends on that.
    """
    if value is None:
        return ()
    seen: dict[str, None] = {}
    for message_id in _MESSAGE_ID.findall(unfold(value)):
        seen.setdefault(message_id, None)
    return tuple(seen)


def decode_subject(value: str | None) -> str:
    """Decode an RFC 2047 encoded-word subject to plain text."""
    if value is None:
        return ""
    return str(make_header(decode_header(unfold(value))))


def parse_date(value: str | None) -> datetime | None:
    """Parse a Date header into an aware UTC datetime."""
    if value is None:
        return None
    try:
        parsed = parsedate_to_datetime(unfold(value))
    except (TypeError, ValueError) as exc:
        raise HeaderError(f"unparsable Date header: {value!r}") from exc
    if parsed.tzinfo is None:
        # RFC 2822 permits -0000, which means "local time, zone unknown".
        # Treating it as UTC is what every other mail store does.
        parsed = parsed.replace(tzinfo=timezone.utc)
    return parsed.astimezone(timezone.utc)


def parse(headers: dict[str, str]) -> ParsedHeaders:
    """Parse the headers we care about out of a case-insensitive mapping."""
    lookup = {key.lower(): value for key, value in headers.items()}

    to_and_cc = [lookup.get("to", ""), lookup.get("cc", "")]
    recipients = tuple(
        address for _, address in getaddresses(to_and_cc) if address
    )
    senders = getaddresses([lookup.get("from", "")])

    return ParsedHeaders(
        message_id=parse_message_id(lookup.get("message-id")),
        in_reply_to=parse_message_id(lookup.get("in-reply-to")),
        references=parse_references(lookup.get("references")),
        sender=senders[0][1] if senders and senders[0][1] else None,
        recipients=recipients,
        subject=decode_subject(lookup.get("subject")),
        date=parse_date(lookup.get("date")),
    )


def thread_parent(parsed: ParsedHeaders) -> str | None:
    """Return the Message-ID this message is a reply to, if any.

    In-Reply-To wins when it is present and well formed; otherwise the last
    entry of References is the immediate parent. Mail clients disagree about
    which they populate, and several populate both inconsistently.
    """
    if parsed.in_reply_to:
        return parsed.in_reply_to
    if parsed.references:
        return parsed.references[-1]
    return None

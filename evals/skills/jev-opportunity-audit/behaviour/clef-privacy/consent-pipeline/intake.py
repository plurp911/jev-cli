"""Synthetic incumbent and a destination-specific consent gate."""

def route(record):
    body = record["body"].lower()
    if "invoice" in body or "refund" in body:
        return "billing"
    if "login" in body or "password" in body:
        return "access"
    return "other"

def may_send(approved, recipient, fields):
    return approved and recipient == "Cloudflare" and set(fields) <= {"body"}

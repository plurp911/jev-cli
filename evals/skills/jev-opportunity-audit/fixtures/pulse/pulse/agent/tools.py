"""Tool definitions for the assistant.

These are the function schemas handed to the model. Adding one here is the
only step needed to expose it; ``pulse.agent.loop`` looks up the handler by
name at call time.

Keep the descriptions short. The whole list is serialized into every request,
so a paragraph here is a paragraph on every turn of every conversation.
"""

from __future__ import annotations

from typing import Any

TOOL_SCHEMAS: list[dict[str, Any]] = [
    {
        "name": "lookup_order",
        "description": "Fetch an order by its order number.",
        "input_schema": {
            "type": "object",
            "properties": {"order_number": {"type": "string"}},
            "required": ["order_number"],
        },
    },
    {
        "name": "list_customer_orders",
        "description": "List a customer's orders, most recent first.",
        "input_schema": {
            "type": "object",
            "properties": {
                "customer_id": {"type": "string"},
                "limit": {"type": "integer", "default": 10},
            },
            "required": ["customer_id"],
        },
    },
    {
        "name": "track_shipment",
        "description": "Get carrier tracking events for a shipment.",
        "input_schema": {
            "type": "object",
            "properties": {"tracking_number": {"type": "string"}},
            "required": ["tracking_number"],
        },
    },
    {
        "name": "lookup_invoice",
        "description": "Fetch an invoice, including line items and tax.",
        "input_schema": {
            "type": "object",
            "properties": {"invoice_id": {"type": "string"}},
            "required": ["invoice_id"],
        },
    },
    {
        "name": "list_charges",
        "description": "List card charges and their status for a customer.",
        "input_schema": {
            "type": "object",
            "properties": {
                "customer_id": {"type": "string"},
                "since": {"type": "string", "description": "ISO 8601 date"},
            },
            "required": ["customer_id"],
        },
    },
    {
        "name": "estimate_refund",
        "description": (
            "Calculate what a refund for an order would come to, including "
            "restocking fees. Does not issue anything."
        ),
        "input_schema": {
            "type": "object",
            "properties": {
                "order_number": {"type": "string"},
                "line_item_ids": {"type": "array", "items": {"type": "string"}},
            },
            "required": ["order_number"],
        },
    },
    {
        "name": "search_knowledge_base",
        "description": "Search help center articles and internal runbooks.",
        "input_schema": {
            "type": "object",
            "properties": {"query": {"type": "string"}},
            "required": ["query"],
        },
    },
    {
        "name": "get_return_policy",
        "description": "Return the policy text that applies to a product category.",
        "input_schema": {
            "type": "object",
            "properties": {"category": {"type": "string"}},
            "required": ["category"],
        },
    },
    {
        "name": "check_stock",
        "description": "Current stock level and restock date for a product.",
        "input_schema": {
            "type": "object",
            "properties": {"sku": {"type": "string"}},
            "required": ["sku"],
        },
    },
    {
        "name": "get_conversation_history",
        "description": "Earlier conversations with the same customer.",
        "input_schema": {
            "type": "object",
            "properties": {
                "customer_id": {"type": "string"},
                "limit": {"type": "integer", "default": 5},
            },
            "required": ["customer_id"],
        },
    },
    {
        "name": "add_internal_note",
        "description": (
            "Attach a note to the conversation. Visible to agents only, never "
            "to the customer."
        ),
        "input_schema": {
            "type": "object",
            "properties": {
                "conversation_id": {"type": "string"},
                "body": {"type": "string"},
            },
            "required": ["conversation_id", "body"],
        },
    },
]

TOOL_NAMES = frozenset(schema["name"] for schema in TOOL_SCHEMAS)

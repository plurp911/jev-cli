"""Role and scope checks.

This is the authorization boundary. Every mutating endpoint calls ``require``
before it touches anything, and the assistant's tool handlers call it with
the acting agent's principal rather than their own. A check that returns True
when it should not is a data breach, so the rules here are explicit, closed
by default, and covered by test_permissions.py line by line.
"""

from __future__ import annotations

from dataclasses import dataclass
from enum import StrEnum


class Role(StrEnum):
    VIEWER = "viewer"
    AGENT = "agent"
    LEAD = "lead"
    ADMIN = "admin"
    OWNER = "owner"


class Scope(StrEnum):
    CONVERSATION_READ = "conversation:read"
    CONVERSATION_WRITE = "conversation:write"
    CONVERSATION_DELETE = "conversation:delete"
    CUSTOMER_READ = "customer:read"
    CUSTOMER_PII_READ = "customer:pii:read"
    REFUND_ISSUE = "refund:issue"
    BILLING_MANAGE = "billing:manage"
    MEMBER_MANAGE = "member:manage"
    SETTINGS_MANAGE = "settings:manage"
    AUDIT_LOG_READ = "audit:read"


# Scopes granted by a role. Roles do not inherit implicitly; each set is
# written out so that widening one role cannot widen another by accident.
ROLE_SCOPES: dict[Role, frozenset[Scope]] = {
    Role.VIEWER: frozenset({Scope.CONVERSATION_READ, Scope.CUSTOMER_READ}),
    Role.AGENT: frozenset(
        {
            Scope.CONVERSATION_READ,
            Scope.CONVERSATION_WRITE,
            Scope.CUSTOMER_READ,
        }
    ),
    Role.LEAD: frozenset(
        {
            Scope.CONVERSATION_READ,
            Scope.CONVERSATION_WRITE,
            Scope.CONVERSATION_DELETE,
            Scope.CUSTOMER_READ,
            Scope.CUSTOMER_PII_READ,
            Scope.REFUND_ISSUE,
        }
    ),
    Role.ADMIN: frozenset(
        {
            Scope.CONVERSATION_READ,
            Scope.CONVERSATION_WRITE,
            Scope.CONVERSATION_DELETE,
            Scope.CUSTOMER_READ,
            Scope.CUSTOMER_PII_READ,
            Scope.REFUND_ISSUE,
            Scope.MEMBER_MANAGE,
            Scope.SETTINGS_MANAGE,
            Scope.AUDIT_LOG_READ,
        }
    ),
    Role.OWNER: frozenset(Scope),
}


class PermissionDenied(Exception):
    """Raised when a principal may not perform an action."""


@dataclass(frozen=True)
class Principal:
    user_id: str
    workspace_id: str
    role: Role
    # Set for API tokens, which may hold fewer scopes than the user's role
    # would otherwise grant. None means "whatever the role grants".
    token_scopes: frozenset[Scope] | None = None
    suspended: bool = False


def granted_scopes(principal: Principal) -> frozenset[Scope]:
    """The scopes a principal effectively holds."""
    if principal.suspended:
        return frozenset()
    role_scopes = ROLE_SCOPES[principal.role]
    if principal.token_scopes is None:
        return role_scopes
    # A token can only ever narrow. Intersecting rather than unioning is the
    # whole point: a leaked token must not outrank its owner.
    return role_scopes & principal.token_scopes


def has_permission(
    principal: Principal, scope: Scope, *, workspace_id: str
) -> bool:
    """Whether ``principal`` may exercise ``scope`` in ``workspace_id``."""
    if principal.workspace_id != workspace_id:
        return False
    return scope in granted_scopes(principal)


def require(principal: Principal, scope: Scope, *, workspace_id: str) -> None:
    """Raise ``PermissionDenied`` unless the principal holds ``scope``."""
    if not has_permission(principal, scope, workspace_id=workspace_id):
        raise PermissionDenied(
            f"{principal.user_id} lacks {scope} in workspace {workspace_id}"
        )

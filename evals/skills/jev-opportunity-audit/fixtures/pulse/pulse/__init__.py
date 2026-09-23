"""Pulse, a shared inbox for support teams.

Nothing heavy belongs in this module. The API process imports ``pulse`` very
early to read ``__version__`` for the ``X-Pulse-Version`` response header, and
the Alembic environment imports it before the database is reachable, so any
import that opens a socket here will break migrations on a cold box.
"""

__version__ = "2.11.3"

# Bumped by hand at release time. The tag in the deploy repository has to match
# or the smoke test after rollout fails on the version header.
__all__ = ["__version__"]

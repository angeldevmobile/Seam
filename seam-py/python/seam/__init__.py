"""Seam: one schema, every language, no drift.

    from seam import Schema

    schema = Schema.load("contracts/user.seam")
    user = schema.validate("User", payload)

`validate` returns a plain dict. That is deliberate: a dict models the four
presence states natively, where an object with attributes cannot. Absence is a
missing key, null is a present key holding None:

    "bio" in user            # was the key sent at all?
    user["avatar"] is None   # was it sent as null?

Static types come from the same .seam file through a separate path, `seam
typegen`, which emits TypedDicts. The generated file holds no rules, so the
schema stays the only source of truth.
"""

from ._seam import (
    Issue,
    Limits,
    ParseError,
    Schema,
    ValidationError,
    Validator,
    __version__,
)

__all__ = [
    "Schema",
    "Validator",
    "Limits",
    "Issue",
    "ValidationError",
    "ParseError",
    "__version__",
]

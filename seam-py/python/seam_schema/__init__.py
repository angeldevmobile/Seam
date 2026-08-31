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

from __future__ import annotations

from ._seam import (
    Issue,
    Issues,
    Limits,
    ParseError,
    Schema,
    Validator,
    __version__,
)


class ValidationError(Exception):
    """Raised when a payload does not satisfy its schema.

    Nothing here is built when the error is raised. The engine hands over one
    object holding the failures, and the path strings, the `Issue` objects and
    the message are produced the first time they are read. A caller that only
    lets the exception propagate pays for none of them.

    There is deliberately no `__init__`: `BaseException` already stores the
    argument in C, and overriding it would put a Python-level call back on the
    raise path, which is the cost this shape exists to avoid.
    """

    __slots__ = ()

    @property
    def _raw(self) -> Issues:
        return self.args[0]

    @property
    def issues(self) -> list[Issue]:
        """Every failure from the pass, in declaration order, depth first."""
        return self._raw.list()

    @property
    def path(self) -> str:
        """The first issue's location, for the common case of reading one."""
        return self._raw.first_path()

    @property
    def code(self) -> str:
        return self._raw.first_code()

    @property
    def message(self) -> str:
        return self._raw.first_message()

    def __len__(self) -> int:
        return len(self._raw)

    def __str__(self) -> str:
        return self._raw.summary()

    def __repr__(self) -> str:
        return f"ValidationError({self._raw.summary()!r})"

__all__ = [
    "Schema",
    "Validator",
    "Limits",
    "Issue",
    "Issues",
    "ValidationError",
    "ParseError",
    "__version__",
]

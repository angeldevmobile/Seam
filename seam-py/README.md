# seam-schema

Python bindings for [Seam](https://github.com/angeldevmobile/Seam): one schema,
every language, no drift.

```python
from seam_schema import Schema

schema = Schema.load("contracts/user.seam")
user = schema.validate("User", payload)
```

`validate` returns a plain dict, deliberately. A dict models Seam's four
presence states natively, where an object with attributes cannot: absence is a
missing key, null is a present key holding `None`.

```python
"bio" in user            # was the key sent at all?
user["avatar"] is None   # was it sent as null?
```

Values come back normalised: a `Date` is a `datetime.date`, a `DateTime` is an
aware `datetime`, and a `u64` keeps every one of its bits.

## Errors

```python
from seam_schema import ValidationError

try:
    schema.validate("User", payload)
except ValidationError as e:
    e.path, e.code, e.message   # the first issue
    for issue in e.issues:      # all of them; validation does not stop at one
        print(issue.path, issue.code)
```

`path` and `code` are stable API. `message` is for humans.

## Limits

Untrusted input is bounded in the engine, so the defaults apply whether or not
you ask. Tighten them to what a legitimate request actually looks like:

```python
from seam_schema import Limits

schema.validate("User", payload, Limits(max_items=100, max_string_bytes=4096))
```

## Static types

A schema loaded at runtime is opaque to a type checker, by construction. So the
`.seam` file feeds two paths: validation at runtime, and types through
`seam typegen`.

```bash
seam typegen contracts/user.seam        # writes contracts/user_types.py
```

```python
from contracts.user_types import validate_user

user = validate_user(payload)
user["name"]        # mypy knows this is str
user["nmae"]        # mypy rejects it
```

The four presence states land on Python's own vocabulary for them, which is why
`TypedDict` was the right target:

| `.seam` | generated |
|---|---|
| `String` | `name: str` |
| `String?` | `nickname: str \| None` |
| `optional String` | `bio: NotRequired[str]` |
| `optional String?` | `avatar: NotRequired[str \| None]` |

**The generated file holds no rules.** Delete it and everything still runs; you
only lose static checking. That is the test that it is not a second source of
truth.

Keep it honest in CI:

```bash
seam typegen --check contracts/*.seam   # fails if stale or missing
```

## Status

```bash
pip install seam-schema
```

Early development, published at `0.1.1`. One `abi3` wheel per platform covers
Python 3.9 and up. Wheels ship for Linux x64, macOS ARM and Windows x64; on any
other platform the install fails rather than falling back, because there is
nothing to fall back to.

Build from a checkout:

```bash
pip install ./seam-py
pytest seam-py/tests
```

# seam

Python bindings for [Seam](https://github.com/angeldevmobile/Seam): one schema,
every language, no drift.

```python
from seam import Schema

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
from seam import ValidationError

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
from seam import Limits

schema.validate("User", payload, Limits(max_items=100, max_string_bytes=4096))
```

## Status

Early development, not yet on PyPI. Build from a checkout:

```bash
pip install ./seam-py
pytest seam-py/tests
```

Static types via generated `TypedDict`s (`seam typegen`) are not implemented
yet. See the repository README for the roadmap.

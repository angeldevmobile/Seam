# Benchmarks

```bash
pip install ./seam-py msgspec pydantic
python benchmarks/bench.py     # seam vs msgspec vs pydantic
python benchmarks/profile.py   # where seam's own time goes
python benchmarks/ab.py        # measure one change, old path vs new
```

Every number here is reproducible with those commands. `bench.py` prints its
own environment, and `--json` writes the raw samples.

## Results, 2026-08-30

Median nanoseconds per validation of an already-parsed dict. Lower is better.

| scenario | seam | msgspec | pydantic v2 |
|---|---:|---:|---:|
| flat, 6 fields | 1 919 | **344** | 1 662 |
| nested + date | 3 858 | **498** | 2 891 |
| array of 100 strings | 10 419 | **862** | 3 011 |
| rejected payload | 3 655 | **957** | 2 145 |

Relative to the fastest in each row:

| scenario | seam | msgspec | pydantic v2 |
|---|---:|---:|---:|
| flat, 6 fields | 5.6x | 1.00x | 4.8x |
| nested + date | 7.7x | 1.00x | 5.8x |
| array of 100 strings | 12.1x | 1.00x | 3.5x |
| rejected payload | 3.8x | 1.00x | 2.2x |

Environment: CPython 3.13.2, Windows 11 (10.0.26200), Intel64 Family 6 Model
140 Stepping 1, seam 0.0.0 (release build, abi3), msgspec 0.21.1, pydantic
2.13.5. Nine repeats per cell, each at least 50 ms, median reported.

Across three runs the flat row came out at 1 919, 2 339 and 1 902 ns against
pydantic's 1 662, 1 778 and 1 756, so **Seam is somewhere between 1.1x and 1.3x
pydantic's cost on a flat payload**, not at parity with it. Every row is still
slower than both competitors.

## Why cross-run comparison is not allowed here

Between two sessions on this laptop, msgspec's array row moved from 1 369 ns to
817 ns without a line of its code changing. A 40% swing in an untouched
baseline means the machine state differs more than most optimisations do.

So an improvement is never claimed by comparing today's table to yesterday's.
It is measured with `ab.py`, which runs the old and new paths against each
other in one process, alternating rounds so drift lands on both.

## What is measured

Validation of a Python dict that has already been parsed. JSON parsing is out
of scope on purpose: it is a different question, and including it would let a
library win on its parser rather than its validator.

All three are asked to do the same work, as closely as they allow:

- the same constraints: length bounds, a numeric range, a closed enum,
- rejecting unknown keys (`forbid_unknown_fields` / `extra="forbid"` / Seam's
  default),
- converting a date string into a `datetime.date`.

Seam is measured through a validator **bound once**, because that is the fair
comparison: msgspec's `Struct` and pydantic's `BaseModel` are also defined once,
before the clock starts.

`bench.py` refuses to report anything until it has confirmed each library
actually accepts the valid payloads and rejects the invalid one. A benchmark of
three libraries quietly doing different work is worse than no benchmark.

## Where they differ, and it matters

- **seam** returns a `dict`, **msgspec** a `Struct`, **pydantic** a model
  instance. Those are not the same amount of construction work.
- **seam** range-checks `id` against `u64`, because width is part of its type.
  The other two treat it as an unbounded `int` and skip that check.
- **msgspec** is a C extension whose fields are slots in a compiled layout.
  Seam's schema is loaded at runtime, so a field is found by name. That is the
  permanent price of the schema being a portable file, and it is the reason
  matching msgspec is not the target.

## Where seam's own time goes

`profile.py` separates the cost that does not depend on the payload from the
cost that scales with it.

| | before any work | now |
|---|---:|---:|
| fixed, per call | ~1 100 ns | **~250 ns** |
| per field | ~440 ns | **~300-330 ns** |
| per array element | ~123-163 ns | **~95-107 ns** |

For scale, a pure-Python dict comprehension over the same six keys costs about
380 ns, or ~63 ns per key. Seam is still roughly five times that per field.

## Progress

### 1. Bind the validator once — done

`schema.validator("User")` resolves the type, the `datetime` classes and the
limits at bind time instead of per call. Measured with `ab.py`, interleaved:

| scenario | unbound | bound | saved |
|---|---:|---:|---:|
| flat | 3 420 | 2 719 | **701 ns** (1.26x) |
| nested + date | 5 568 | 4 746 | **822 ns** (1.17x) |
| array of 100 | 15 492 | 14 501 | **991 ns** (1.07x) |
| rejected | 5 400 | 4 276 | **1 124 ns** (1.26x) |

The saving is roughly constant at 700 to 1 100 ns whatever the payload, which
is the signature of a fixed cost being removed. It is worth 26% on a flat
payload and 7% on an array of 100, because a fixed cost matters less the more
data there is.

`schema.validate(type, payload)` still exists and still works; it now builds a
validator and discards it, which is exactly the "unbound" column above.

### 2. Validate directly against the host's objects — done

`seam-core::validate` is generic over an `Input` trait. `Value` implements it,
and `seam-py` implements it for `Bound<'py, PyAny>`, so no intermediate copy is
built. Values that need no conversion are handed back as the very object that
arrived, rather than rebuilt.

It delivered where it was aimed and not elsewhere:

- **array elements: ~35% cheaper.** The copy really was the cost there.
- **object fields: ~12% cheaper.** Barely anything.

That gap is the finding. An object field is no longer paying for a copy, so
what remains must be something else, and it is: **two Python strings are created
per field per call**, one to look the key up in the input and one to write it
into the output. Twelve string objects for a six-field payload. That is step 3
below, which this measurement promotes from polish to the main remaining win for
object payloads.

The rejected row also refused to move, and for its own reason: at 4 312 ns it is
*more expensive than the valid path*, because nothing is built for the result
but an exception is. Constructing `ValidationError`, the list, the `Issue`
objects and their attributes costs more than validating six fields. Seam's error
contract is richer than msgspec's message string, and this is its price;
building the issues lazily is the obvious lever and has not been tried.

### 3. Stop paying for the error path on the happy path — done

Two changes, measured separately.

**A borrowed path, in `seam-core`.** The validator used to clone a field's name
into an owned `Segment` for every field of every payload, so that a path would
be ready if something went wrong. On a valid payload nothing ever read it. The
path now borrows from the schema and is materialised only when an issue is
actually recorded. Worth ~380 to ~365 ns per field and ~95 to ~87 ns per array
element, and it lands in the core, so every binding gets it.

**Interned keys, in `seam-py`.** Each declared field name becomes one interned
Python string at bind time, used both to look the key up in the payload and to
write it into the result. It replaces two fresh string objects per field per
call, and an interned string carries its hash already computed. Worth ~365 to
~300-330 ns per field. Array elements did not move, which is right: they have no
keys.

Together the flat row went from roughly 2 800 to roughly 1 900 ns.

### 4. Build validation issues lazily — next

The rejected row is still *more expensive than the valid path*: nothing is built
for the result, but `ValidationError`, a list, the `Issue` objects and their
attributes are. Seam's error contract is richer than a message string and this
is its price. Deferring the `Issue` objects until `.issues` is read would keep
the contract and stop charging for it up front.

### 5. Cheaper classification

`classify` walks up to seven `is_instance_of` checks per value. Ordering them by
what payloads actually contain, or checking the type object once, is untested
ground.

## Caveats

- One machine, one OS, one CPython. A Linux or macOS run may differ.
- Windows timer granularity is coarser than Linux's; the calibration loop
  targets 50 ms per repeat to stay well clear of it.
- No attempt was made to pin CPU frequency or isolate cores. **Read the ratios,
  and only their first digit.** The claim supported here is "Seam is roughly an
  order of magnitude behind msgspec and under twice pydantic's cost on flat
  payloads", not any particular nanosecond count.

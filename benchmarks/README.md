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
| flat, 6 fields | 2 825 | **373** | 1 688 |
| nested + date | 4 852 | **505** | 2 712 |
| array of 100 strings | 10 769 | **829** | 3 050 |
| rejected payload | 4 312 | **1 133** | 2 040 |

Relative to the fastest in each row:

| scenario | seam | msgspec | pydantic v2 |
|---|---:|---:|---:|
| flat, 6 fields | 7.6x | 1.00x | 4.5x |
| nested + date | 9.6x | 1.00x | 5.4x |
| array of 100 strings | 13.0x | 1.00x | 3.7x |
| rejected payload | 3.8x | 1.00x | 1.8x |

Environment: CPython 3.13.2, Windows 11 (10.0.26200), Intel64 Family 6 Model
140 Stepping 1, seam 0.0.0 (release build, abi3), msgspec 0.21.1, pydantic
2.13.5. Nine repeats per cell, each at least 50 ms, median reported. Two
consecutive runs agreed within a few percent.

**Seam is the slowest of the three in every scenario.** That is still the
finding.

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

| | before any work | after steps 1 and 2 |
|---|---:|---:|
| fixed, per call | ~1 100 ns | **~280 ns** |
| per field | ~440 ns | **~390 ns** |
| per array element | ~123-163 ns | **~91-98 ns** |

For scale, a pure-Python dict comprehension over the same six keys costs about
380 ns, or ~63 ns per key. Seam is still six times that per field.

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

### 3. Cache interned `Py<PyString>` keys per field — next

A field's name is known when the validator is bound, so the Python string for it
can be built once instead of twice per call. Interned strings also carry a
precomputed hash. Promoted above the item below on the strength of the
measurement in step 2.

### 4. Build validation issues lazily

The rejected path pays full price for a structured error even when the caller
only reads `str(e)`. Deferring the `Issue` objects until `.issues` is touched
would leave the contract intact and stop charging for it up front.

Whether Seam ends up ahead of pydantic v2 has not been measured and will not be
guessed at here. It is now within 1.8x on the rejected row and 4.5x on flat.

## Caveats

- One machine, one OS, one CPython. A Linux or macOS run may differ.
- Windows timer granularity is coarser than Linux's; the calibration loop
  targets 50 ms per repeat to stay well clear of it.
- No attempt was made to pin CPU frequency or isolate cores. **Read the ratios,
  and only their first digit.** The claim supported here is "Seam is roughly an
  order of magnitude behind msgspec and under twice pydantic's cost on flat
  payloads", not any particular nanosecond count.

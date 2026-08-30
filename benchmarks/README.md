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
| flat, 6 fields | 2 704 | **331** | 1 665 |
| nested + date | 4 938 | **501** | 2 847 |
| array of 100 strings | 15 906 | **817** | 2 964 |
| rejected payload | 4 054 | **907** | 1 970 |

Relative to the fastest in each row:

| scenario | seam | msgspec | pydantic v2 |
|---|---:|---:|---:|
| flat, 6 fields | 8.2x | 1.00x | 5.0x |
| nested + date | 9.9x | 1.00x | 5.7x |
| array of 100 strings | 19.5x | 1.00x | 3.6x |
| rejected payload | 4.5x | 1.00x | 2.2x |

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

From `profile.py`, before any optimisation:

```
 1 field :  1558 ns
 6 fields:  3745 ns   (+437 ns/field)
12 fields:  7058 ns   (+552 ns/field)

dict(p), 6 keys      :  93 ns
dict comprehension   : 431 ns   (~72 ns/key)
```

Two findings. About **1 100 ns was fixed cost**, paid before touching any data:
msgspec's entire flat validation is 344 ns, less than a third of that. And the
**~440 ns per field** is roughly six times what rebuilding the dict costs in
pure Python, because a `String` is copied Python to Rust and back: two
allocations and two memcpys for bytes that never changed.

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

### 2. Validate directly against the host's objects — next

Make `seam-core` generic over an input trait that `Value` implements and each
binding implements for its own runtime, so no intermediate copy is built. This
is what `pydantic-core` does. The array row barely moved in step 1, which
confirms its cost is per element, and this is what addresses that.

### 3. Skip rebuilding the output when nothing needs normalising

A schema with no `Date` or `DateTime` field produces an output dict identical to
the input, built key by key for nothing. Aliasing the input instead of copying
it is a semantic decision to take deliberately, not a micro-optimisation to
sneak in.

### 4. Cache interned `Py<PyString>` keys per field

Rather than creating a Python string for every key on every call. Interned
strings carry a precomputed hash, so this is a win rather than a wash, but it
will be measured with `ab.py` like everything else.

Whether Seam ends up ahead of pydantic v2 after 2 and 3 has not been measured
and will not be guessed at here.

## Caveats

- One machine, one OS, one CPython. A Linux or macOS run may differ.
- Windows timer granularity is coarser than Linux's; the calibration loop
  targets 50 ms per repeat to stay well clear of it.
- No attempt was made to pin CPU frequency or isolate cores. **Read the ratios,
  and only their first digit.** The claim supported here is "Seam is roughly an
  order of magnitude behind msgspec and under twice pydantic's cost on flat
  payloads", not any particular nanosecond count.

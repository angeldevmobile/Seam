# Benchmarks

```bash
pip install ./seam-py msgspec pydantic
python benchmarks/bench.py
```

Every number here is reproducible with that command. `bench.py` prints its own
environment, and `--json` writes the raw samples.

## Results, 2026-08-30

Median nanoseconds per validation of an already-parsed dict. Lower is better.

| scenario | seam | msgspec | pydantic v2 |
|---|---:|---:|---:|
| flat, 6 fields | 3 671 | **344** | 1 826 |
| nested + date | 6 032 | **938** | 3 390 |
| array of 100 strings | 24 360 | **1 369** | 4 273 |
| rejected payload | 8 531 | **1 218** | 3 084 |

Relative to the fastest in each row:

| scenario | seam | msgspec | pydantic v2 |
|---|---:|---:|---:|
| flat, 6 fields | 10.67x | 1.00x | 5.31x |
| nested + date | 6.43x | 1.00x | 3.61x |
| array of 100 strings | 17.80x | 1.00x | 3.12x |
| rejected payload | 7.01x | 1.00x | 2.53x |

Environment: CPython 3.13.2, Windows 11 (10.0.26200), Intel64 Family 6 Model
140 Stepping 1, seam 0.0.0 (release build, abi3), msgspec 0.21.1, pydantic
2.13.5. Seven repeats per cell, each at least 50 ms, median reported.

**Seam is the slowest of the three in every scenario.** That is the finding.

## What is measured

Validation of a Python dict that has already been parsed. JSON parsing is out
of scope on purpose: it is a different question, and including it would let a
library win on its parser rather than its validator.

All three are asked to do the same work, as closely as they allow:

- the same constraints: length bounds, a numeric range, a closed enum,
- rejecting unknown keys (`forbid_unknown_fields` / `extra="forbid"` / Seam's
  default),
- converting a date string into a `datetime.date`.

`bench.py` refuses to report anything until it has confirmed each library
actually accepts the valid payloads and rejects the invalid one. A benchmark of
three libraries quietly doing different work is worse than no benchmark.

## Where they differ, and it matters

- **seam** returns a `dict`, **msgspec** a `Struct`, **pydantic** a model
  instance. Those are not the same amount of construction work, and no
  arrangement of this benchmark makes them so.
- **seam** range-checks `id` against `u64`, because width is part of its type.
  The other two treat it as an unbounded `int` and skip that check.
- **msgspec** is a C extension building a struct with a fixed layout. Beating it
  inside a single language was never the goal, and the project README says so.

## Reading the shape, not just the size

The interesting number is not `flat`. It is these two:

**`array of 100` is the worst at 17.8x.** Cost scales with element count far
worse than the others, which points at per-element allocation rather than a
fixed overhead.

**`rejected` is 7x, and it should have been Seam's best row.** A rejected
payload never needs an output object, so this is where "the work stays on the
Rust side" ought to pay. It does not pay at all, and that says exactly where the
time goes.

## The diagnosis

`Schema.validate` materialises the payload three times:

```
dict de Python  ->  seam_core::Value  ->  dict de Python
   (input)          (a full copy)          (output)
```

The middle representation exists only because `seam_core::validate` takes a
`&Value`. It is allocated, walked once, and dropped. On the rejected path it is
built in full before a single rule runs, which is why rejection is not cheap.

The main README claimed Seam "walks the input directly instead of building an
intermediate object graph to throw away". That was false, and this benchmark is
what caught it. The claim has been corrected rather than the benchmark
massaged.

## The fix, in order of expected value

1. **Validate directly against the host's objects.** Make `seam-core` generic
   over an input trait that `Value` implements and each binding implements for
   its own runtime, so no intermediate copy is built. This is what
   `pydantic-core` does. It should take most of the gap on the array and
   rejected rows.
2. **Skip rebuilding the output when nothing needs normalising.** A schema with
   no `Date` or `DateTime` field produces an output dict identical to the input,
   built key by key for nothing. Precomputing "does this type need conversion"
   would remove that pass entirely for the `flat` shape. Aliasing versus copying
   is a semantic decision to make deliberately, not a micro-optimisation to
   sneak in.
3. **Hoist the `datetime` import.** `Ctx::new` imports the module and reads two
   attributes on every call. Small, but it is paid per validation and belongs on
   the schema.

Whether Seam ends up competitive with pydantic v2 after (1) and (2) is an open
question, and the honest answer is that it has not been measured yet. Matching
msgspec is not the target.

## Caveats

- One machine, one OS, one CPython. A Linux or macOS run may differ.
- Windows timer granularity is coarser than Linux's; the calibration loop
  targets 50 ms per repeat to stay well clear of it.
- No attempt was made to pin CPU frequency or isolate cores. Across two runs on
  an idle laptop, individual cells moved by up to 50% (`rejected` for msgspec
  came out at 1 218 ns and then 1 828 ns), while Seam's own numbers held within
  a few percent. **Read the ratios, and only their first digit.** The claim
  supported here is "Seam is roughly an order of magnitude behind msgspec and
  about twice pydantic's cost", not any particular nanosecond count.

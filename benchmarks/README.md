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

### From an already-parsed dict

This isolates the validator, which is what the optimisation work below targets.

| scenario | seam | msgspec | pydantic v2 |
|---|---:|---:|---:|
| flat, 6 fields | 2 355 | **341** | 1 666 |
| nested + date | 3 729 | **500** | 2 735 |
| array of 100 strings | 9 780 | **795** | 2 911 |
| rejected payload | 2 351 | **940** | 1 967 |

| scenario | seam | msgspec | pydantic v2 |
|---|---:|---:|---:|
| flat, 6 fields | 6.9x | 1.00x | 4.9x |
| nested + date | 7.5x | 1.00x | 5.5x |
| array of 100 strings | 12.3x | 1.00x | 3.7x |
| rejected payload | 2.5x | 1.00x | 2.1x |

### From raw JSON bytes, as a request actually arrives

All three parse and validate in one call. This is the table to quote when
someone asks how fast Seam is.

| scenario | seam | msgspec | pydantic v2 |
|---|---:|---:|---:|
| flat, 6 fields | 1 924 | **421** | 1 589 |
| nested + date | 3 660 | **657** | 2 559 |
| array of 100 strings | 13 748 | **3 685** | 5 566 |

| scenario | seam | msgspec | pydantic v2 |
|---|---:|---:|---:|
| flat, 6 fields | 4.6x | 1.00x | 3.8x |
| nested + date | 5.6x | 1.00x | 3.9x |
| array of 100 strings | 3.7x | 1.00x | 1.5x |

Seam is now *faster from bytes than from a dict*, the same shape pydantic has,
because parsing straight into the result skips building a dict nobody asked for.

Before Seam owned the parse, this table read 4 668, 7 069 and 15 685 ns, or
2.8-3.0x pydantic. The flat row is now within 1.2x of it.

The array row barely moved and is the one left: its cost is emitting a hundred
Python strings, which no parser change reaches.

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

Two things, reported separately because they answer different questions. The
first isolates the validator. The second is what a service holds when a request
lands, and it is the one to quote when someone asks how fast Seam is.

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

### 4. Build validation issues lazily — done

Measured on a two-field schema, valid versus rejected in the same process:

| | before | after |
|---|---:|---:|
| cost of the exception | 1 802 ns | **601 ns** |

A bare `raise`/`except` of a Python exception with no attributes costs 206 ns on
this machine, so what remains above that floor is now about 395 ns.

The exception is declared in Python and overrides no `__init__`, so raising it
is C-level bookkeeping plus one allocation on the Rust side. Everything else —
the path strings, the `Issue` objects, the summary — is produced when read.
A caller that lets the error propagate pays for none of it.

The whole rejected row went from 3 655 to about 2 400 ns, from 1.8x to 1.2x
pydantic.

### 5. Cheaper classification — dropped, it would not help

`classify` walks up to seven `is_instance_of` checks, so ordering them by what
payloads actually contain looked promising. Measuring six fields of a single
type, by that type's position in the chain:

| type | position | per field |
|---|---|---:|
| `bool` | 2nd | 244 ns |
| `int` | 3rd | 266 ns |
| `str` | 4th | 303 ns |
| `float` | 5th | 248 ns |

If position drove the cost, `float` at fifth would be the most expensive. It is
the second cheapest. The spread is explained by the work each type needs — `str`
has to be decoded — and not by how many checks preceded it, so each check must
cost very little. Reordering would buy nothing, and the idea is dropped rather
than implemented on the strength of how plausible it sounded.

## Why Seam parses, which is not about speed

The main README's headline example is `JSON.parse` corrupting
`9007199254740993` into `...992`. If a binding is handed host objects that have
already been parsed, **the corruption happened before Seam saw anything**, and
validating a `number` whose bits are already gone accomplishes nothing.

Python got away with it: `json.loads` uses arbitrary-precision integers, so the
dict path was already correct. That was the language being generous, not a
design decision, and JavaScript is not generous.

So parsing is a correctness requirement for the second binding, and the speed it
happens to buy is a side effect. `seam-core/src/json.rs` is hand-written and adds
no dependency: the crate is loaded into three runtimes, the rules that keep an
integer intact have to run while the bytes are read, and a general-purpose JSON
crate would rebuild the intermediate tree the `Input` trait exists to avoid.

It parses in order to validate. There is no encoder, which is what keeps the
main README's "no general-purpose serialization" line true.

## Caveats

- One machine, one OS, one CPython. A Linux or macOS run may differ.
- Windows timer granularity is coarser than Linux's; the calibration loop
  targets 50 ms per repeat to stay well clear of it.
- No attempt was made to pin CPU frequency or isolate cores. **Read the ratios,
  and only their first digit.** The claim supported here is "Seam is roughly an
  order of magnitude behind msgspec and under twice pydantic's cost on flat
  payloads", not any particular nanosecond count.

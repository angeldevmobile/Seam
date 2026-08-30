"""Validation benchmarks: seam against msgspec and pydantic v2.

Run:

    pip install ./seam-py msgspec pydantic
    python benchmarks/bench.py

Everything the run depends on is printed with the results, and `--json` writes
the same data as a file. A number without its methodology is not a measurement.

What is being measured
----------------------
Two tables.

The first validates an already-parsed Python dict. That isolates the validator,
which is what most of this file's optimisation work targets.

The second starts from raw JSON bytes, which is what a service actually holds.
It matters because msgspec and pydantic both fuse parsing into validation and
never build the intermediate dict at all, while Seam has no such path: it needs
`json.loads` first. Reporting only the first table would quietly hide an
architectural gap behind a favourable measurement.

What each library is asked to do
--------------------------------
The same work, as closely as the three allow:

* the same constraints (length bounds, a numeric range, a closed enum),
* rejecting unknown keys (`forbid_unknown_fields` / `extra="forbid"` /
  Seam's default),
* converting a date string into a `datetime.date`.

Where they differ, and it matters
---------------------------------
* **seam** returns a `dict`; **msgspec** returns a `Struct`; **pydantic**
  returns a model instance. Constructing those is not the same amount of work,
  and no reordering of this benchmark makes it so.
* **seam** range-checks `id` against `u64` because width is part of its type.
  The other two treat `id` as an unbounded `int` and do no such check.
* **seam** currently materialises the payload into an intermediate
  `seam_core::Value` before validating. That copy is the main thing this
  benchmark exists to price.
"""

from __future__ import annotations

import argparse
import datetime
import json
import platform
import statistics
import sys
import time
from pathlib import Path
from typing import Any, Callable, Literal

import msgspec
import pydantic
from msgspec import Meta, Struct
from pydantic import BaseModel, ConfigDict, Field
from typing_extensions import Annotated

import seam

HERE = Path(__file__).resolve().parent
SCHEMA = HERE / "schemas" / "bench.seam"


# --------------------------------------------------------------- the schemas

_seam_schema = seam.Schema.load(SCHEMA)

# Bound once, at import. This is the fair comparison: msgspec's Struct and
# pydantic's BaseModel are also defined once, at class-definition time. Calling
# `schema.validate(type_name, payload)` in the loop would make Seam re-resolve
# per call something the other two resolved before the clock started.
_seam_user = _seam_schema.validator("User")
_seam_nested = _seam_schema.validator("UserNested")
_seam_arrays = _seam_schema.validator("UserArrays")


class MsAddress(Struct, forbid_unknown_fields=True):
    city: Annotated[str, Meta(min_length=1, max_length=64)]
    zip: Annotated[str, Meta(min_length=3, max_length=10)]


class MsUser(Struct, forbid_unknown_fields=True):
    id: int
    name: Annotated[str, Meta(min_length=3, max_length=64)]
    age: Annotated[int, Meta(ge=0, le=130)]
    plan: Literal["free", "pro", "enterprise"]
    active: bool
    score: float


class MsUserNested(Struct, forbid_unknown_fields=True):
    id: int
    name: Annotated[str, Meta(min_length=3, max_length=64)]
    age: Annotated[int, Meta(ge=0, le=130)]
    plan: Literal["free", "pro", "enterprise"]
    active: bool
    score: float
    home: MsAddress
    joined: datetime.date


class MsUserArrays(Struct, forbid_unknown_fields=True):
    id: int
    name: Annotated[str, Meta(min_length=3, max_length=64)]
    tags: Annotated[list[str], Meta(max_length=1000)]


class PyAddress(BaseModel):
    model_config = ConfigDict(extra="forbid")
    city: str = Field(min_length=1, max_length=64)
    zip: str = Field(min_length=3, max_length=10)


class PyUser(BaseModel):
    model_config = ConfigDict(extra="forbid")
    id: int
    name: str = Field(min_length=3, max_length=64)
    age: int = Field(ge=0, le=130)
    plan: Literal["free", "pro", "enterprise"]
    active: bool
    score: float


class PyUserNested(PyUser):
    home: PyAddress
    joined: datetime.date


class PyUserArrays(BaseModel):
    model_config = ConfigDict(extra="forbid")
    id: int
    name: str = Field(min_length=3, max_length=64)
    tags: list[str] = Field(max_length=1000)


# -------------------------------------------------------------- the payloads


def flat() -> dict[str, Any]:
    return {
        "id": 9007199254740993,
        "name": "Gabriel",
        "age": 30,
        "plan": "pro",
        "active": True,
        "score": 91.5,
    }


def nested() -> dict[str, Any]:
    return {**flat(), "home": {"city": "Lima", "zip": "15001"}, "joined": "2026-08-29"}


def arrays(n: int) -> dict[str, Any]:
    return {
        "id": 1,
        "name": "Gabriel",
        "tags": [f"tag-{i}" for i in range(n)],
    }


def invalid() -> dict[str, Any]:
    return {**flat(), "plan": "platinum"}


def as_json(payload: dict[str, Any]) -> bytes:
    return json.dumps(payload).encode()


# ------------------------------------------------------------------ scenarios


def catching(fn: Callable[[Any], Any]) -> Callable[[Any], Any]:
    """Rejection is a real workload: a service that rejects a bad request pays
    for it. All three raise, so all three pay for an exception here."""

    def run(payload: Any) -> Any:
        try:
            return fn(payload)
        except Exception:  # noqa: BLE001 - the failure is the measurement
            return None

    return run


def scenarios() -> dict[str, dict[str, tuple[Callable[[Any], Any], Any]]]:
    return {
        "flat": {
            "seam": (_seam_user, flat()),
            "msgspec": (lambda p: msgspec.convert(p, MsUser), flat()),
            "pydantic": (PyUser.model_validate, flat()),
        },
        "nested + date": {
            "seam": (_seam_nested, nested()),
            "msgspec": (lambda p: msgspec.convert(p, MsUserNested), nested()),
            "pydantic": (PyUserNested.model_validate, nested()),
        },
        "array of 100": {
            "seam": (_seam_arrays, arrays(100)),
            "msgspec": (lambda p: msgspec.convert(p, MsUserArrays), arrays(100)),
            "pydantic": (PyUserArrays.model_validate, arrays(100)),
        },
        "rejected": {
            "seam": (catching(_seam_user), invalid()),
            "msgspec": (catching(lambda p: msgspec.convert(p, MsUser)), invalid()),
            "pydantic": (catching(PyUser.model_validate), invalid()),
        },
    }


def json_scenarios() -> dict[str, dict[str, tuple[Callable[[Any], Any], Any]]]:
    """From raw bytes, the way a request arrives.

    All three parse and validate in one call now; none of them builds an
    intermediate dict the caller asked for.
    """
    return {
        "json flat": {
            "seam": (_seam_user, as_json(flat())),
            "msgspec": (
                lambda b: msgspec.json.decode(b, type=MsUser),
                as_json(flat()),
            ),
            "pydantic": (PyUser.model_validate_json, as_json(flat())),
        },
        "json nested": {
            "seam": (_seam_nested, as_json(nested())),
            "msgspec": (
                lambda b: msgspec.json.decode(b, type=MsUserNested),
                as_json(nested()),
            ),
            "pydantic": (PyUserNested.model_validate_json, as_json(nested())),
        },
        "json array of 100": {
            "seam": (_seam_arrays, as_json(arrays(100))),
            "msgspec": (
                lambda b: msgspec.json.decode(b, type=MsUserArrays),
                as_json(arrays(100)),
            ),
            "pydantic": (PyUserArrays.model_validate_json, as_json(arrays(100))),
        },
    }


# -------------------------------------------------------------------- timing


def calibrate(fn: Callable[[Any], Any], payload: Any, target_ns: int) -> int:
    """Iterations per repeat so one repeat lasts long enough for the clock."""
    n = 1
    while True:
        start = time.perf_counter_ns()
        for _ in range(n):
            fn(payload)
        if time.perf_counter_ns() - start >= target_ns or n >= 1_000_000:
            return n
        n *= 2


def measure(
    fn: Callable[[Any], Any], payload: Any, repeats: int, target_ms: int
) -> list[float]:
    for _ in range(200):  # warm up: import caches, branch predictors, JIT-less
        fn(payload)
    n = calibrate(fn, payload, target_ms * 1_000_000)

    samples = []
    for _ in range(repeats):
        start = time.perf_counter_ns()
        for _ in range(n):
            fn(payload)
        samples.append((time.perf_counter_ns() - start) / n)
    return samples


def check_equivalent() -> list[str]:
    """A benchmark of three libraries doing different work is worthless, so
    confirm each actually accepts the valid payloads and rejects the bad one."""
    problems = []
    for name, entries in {**scenarios(), **json_scenarios()}.items():
        if name == "rejected":
            for lib, (fn, payload) in entries.items():
                if fn(payload) is not None:
                    problems.append(f"{lib} accepted the invalid payload in {name!r}")
            continue
        for lib, (fn, payload) in entries.items():
            try:
                fn(payload)
            except Exception as e:  # noqa: BLE001
                problems.append(f"{lib} rejected the valid payload in {name!r}: {e}")
    return problems


# -------------------------------------------------------------------- report


def environment() -> dict[str, str]:
    return {
        "python": sys.version.split()[0],
        "implementation": platform.python_implementation(),
        "platform": platform.platform(),
        "machine": platform.machine(),
        "processor": platform.processor() or "unknown",
        "seam": seam.__version__,
        "msgspec": msgspec.__version__,
        "pydantic": pydantic.VERSION,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repeats", type=int, default=7)
    parser.add_argument("--target-ms", type=int, default=50)
    parser.add_argument("--json", help="also write the results here")
    args = parser.parse_args()

    problems = check_equivalent()
    if problems:
        print("the libraries are not doing equivalent work:", file=sys.stderr)
        for p in problems:
            print(f"  - {p}", file=sys.stderr)
        return 1

    env = environment()
    print("Environment")
    for k, v in env.items():
        print(f"  {k:<15} {v}")
    print(
        f"\n{args.repeats} repeats, each at least {args.target_ms} ms. "
        "Median ns per validation; lower is better.\n"
    )

    results: dict[str, dict[str, dict[str, float]]] = {}
    libs = ["seam", "msgspec", "pydantic"]

    header = f"{'scenario':<20}" + "".join(f"{lib:>14}" for lib in libs)

    def run_group(title: str, group) -> None:
        print()
        print(title)
        print(header)
        print("-" * len(header))
        for name, entries in group.items():
            row = {}
            for lib in libs:
                fn, payload = entries[lib]
                samples = measure(fn, payload, args.repeats, args.target_ms)
                row[lib] = {
                    "median_ns": statistics.median(samples),
                    "min_ns": min(samples),
                    "stdev_ns": statistics.stdev(samples) if len(samples) > 1 else 0.0,
                }
            results[name] = row
            cells = "".join(f"{row[lib]['median_ns']:>13.0f} " for lib in libs)
            print(f"{name:<20}{cells}")

    run_group("From an already-parsed dict:", scenarios())
    run_group("From raw JSON bytes, as a request arrives:", json_scenarios())

    fastest = min(libs, key=lambda l: results["flat"][l]["median_ns"])
    print(f"\nRelative to {fastest} (1.00 = fastest in that row):\n")
    print(header)
    print("-" * len(header))
    for name, row in results.items():
        best = min(row[lib]["median_ns"] for lib in libs)
        cells = "".join(f"{row[lib]['median_ns'] / best:>13.2f}x" for lib in libs)
        print(f"{name:<16}{cells}")

    if args.json:
        Path(args.json).write_text(
            json.dumps({"environment": env, "results": results}, indent=2),
            encoding="utf-8",
        )
        print(f"\nwrote {args.json}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())

"""Compares two callables in the same process, interleaved.

    python benchmarks/ab.py

Cross-run comparison on a laptop is not trustworthy: between two sessions of
`bench.py`, msgspec's array row moved 40% without its code changing. Measuring
an optimisation therefore means running the old and new paths against each
other in one process, alternating rounds so any drift lands on both.

Today it compares `schema.validate(type, payload)`, which resolves the type and
the datetime classes on every call, against a validator bound once.
"""
import time, statistics, seam
from pathlib import Path

S = seam.Schema.load("benchmarks/schemas/bench.seam")

def timed(fn, p, n):
    t0 = time.perf_counter_ns()
    for _ in range(n): fn(p)
    return (time.perf_counter_ns()-t0)/n

def ab(old, new, p, rounds=11, n=20000):
    for _ in range(1000): old(p); new(p)
    a, b = [], []
    for _ in range(rounds):          # intercalado: la deriva afecta a ambos
        a.append(timed(old, p, n))
        b.append(timed(new, p, n))
    return statistics.median(a), statistics.median(b)

def catching(f):
    def g(p):
        try: return f(p)
        except Exception: return None
    return g

flat   = {"id": 9007199254740993, "name": "Gabriel", "age": 30,
          "plan": "pro", "active": True, "score": 91.5}
nested = {**flat, "home": {"city": "Lima", "zip": "15001"}, "joined": "2026-08-29"}
arrays = {"id": 1, "name": "Gabriel", "tags": [f"tag-{i}" for i in range(100)]}
bad    = {**flat, "plan": "platinum"}

cases = [
    ("flat",          "User",       flat,   False),
    ("nested + date", "UserNested", nested, False),
    ("array of 100",  "UserArrays", arrays, False),
    ("rejected",      "User",       bad,    True),
]

print(f"{'escenario':<16}{'sin ligar':>11}{'ligado':>10}{'ahorro':>10}{'':>8}")
print("-"*55)
for name, ty, payload, is_bad in cases:
    v = S.validator(ty)
    old = (lambda p, t=ty: S.validate(t, p))
    new = v
    if is_bad:
        old, new = catching(old), catching(new)
    nn = 3000 if "array" in name else 20000
    a, b = ab(old, new, payload, n=nn)
    print(f"{name:<16}{a:>10.0f} {b:>9.0f} {a-b:>9.0f} ns  ({a/b:.2f}x)")

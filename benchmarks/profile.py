"""Where the time goes, so optimisation targets are chosen from data.

    python benchmarks/profile.py

Separates the cost that does not depend on the payload from the cost that
scales with it, and prints pure-Python floors to compare against. A marginal
cost per field far above a dict insert is the signal worth acting on.
"""

import time, statistics, seam, tempfile
from pathlib import Path

def bench(fn, arg, repeats=7, n=20000):
    for _ in range(500): fn(arg)
    s = []
    for _ in range(repeats):
        t0 = time.perf_counter_ns()
        for _ in range(n): fn(arg)
        s.append((time.perf_counter_ns()-t0)/n)
    return statistics.median(s)

d = Path(tempfile.mkdtemp())

for k in (1, 6, 12):
    fields = "\n".join(f"  f{i}: String" for i in range(k))
    (d/f"s{k}.seam").write_text(f"schema T {{\n{fields}\n}}\n")

payloads = {k: {f"f{i}": "value" for i in range(k)} for k in (1, 6, 12)}
schemas  = {k: seam.Schema.load(d/f"s{k}.seam").validator("T") for k in (1, 6, 12)}

print("--- seam: costo por cantidad de campos (String, sin reglas) ---")
prev = None
for k in (1, 6, 12):
    ns = bench(schemas[k], payloads[k])
    marg = "" if prev is None else f"   (+{(ns-prev[1])/(k-prev[0]):.0f} ns/campo)"
    print(f"  {k:>2} campos: {ns:>8.0f} ns{marg}")
    prev = (k, ns)

print("\n--- pisos de referencia en Python puro ---")
p6 = payloads[6]
print(f"  dict(p), 6 claves       : {bench(dict, p6):>8.1f} ns")
print(f"  comprension de dict     : {bench(lambda p: {k:v for k,v in p.items()}, p6):>8.1f} ns")
print(f"  len(p), llamada trivial : {bench(len, p6):>8.1f} ns")

print("\n--- arrays: costo por elemento ---")
(d/"arr.seam").write_text("schema T {\n  xs: [String] @max_items(10000)\n}\n")
sa = seam.Schema.load(d/"arr.seam").validator("T")
prev = None
for n in (10, 100, 1000):
    pl = {"xs": [f"t{i}" for i in range(n)]}
    ns = bench(sa, pl, repeats=5, n=2000)
    marg = "" if prev is None else f"   (+{(ns-prev[1])/(n-prev[0]):.1f} ns/elem)"
    print(f"  {n:>5} elem: {ns:>9.0f} ns{marg}")
    prev = (n, ns)

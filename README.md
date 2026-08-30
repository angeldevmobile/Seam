# Seam

**One schema. Every language. No drift.**

Seam is a cross-language data contract: you write the schema once in a `.seam` file, and
Python, Node, the browser, and later the JVM, all validate against the *same compiled engine*,
with the same rules, the same errors, and the same answers on the cases that usually break.

There is no generated mirror type to keep in sync. There is no second validator to drift.

---

## The problem is not validation. It's the boundary.

Every language already has a good validator. Pydantic, Zod, Valibot and Jakarta Bean Validation
all work well *inside* their own language.

The bugs don't happen inside a language. They happen at the seam between two of them, because
each language quietly disagrees about what your data means. Here is one payload:

```json
{ "id": 9007199254740993, "nickname": null, "signup_date": "2026-08-29" }
```

Three runtimes, three different readings, zero errors raised:

| | `id` | `nickname` | `signup_date` |
|---|---|---|---|
| **Python** | `9007199254740993`, exact | `None`, indistinguishable from a missing key | a `str`, unless something parsed it |
| **JavaScript** | `9007199254740992`, **silently wrong** | `null`, but re-serializing after setting it to `undefined` **deletes the key** | `new Date(...)` → UTC midnight; in UTC-5 `.getDate()` returns **28** |
| **Java** | `9007199254740993L`, exact | `null`, indistinguishable from a missing key | `[2026,8,29]` under Jackson's defaults, unless JSR-310 is registered |

Nobody threw. Nobody logged. The ID is corrupted on one side, the date is off by a day on
another, and "user cleared their nickname" is indistinguishable from "client didn't send the
field" on two of the three. This is what a polyglot data bug actually looks like, and it is
why it surfaces three weeks later in production instead of in CI.

### Why each language loses information

- **JavaScript / TypeScript.** Numbers are IEEE-754 doubles: any integer above
  `Number.MAX_SAFE_INTEGER` (2^53 - 1) loses precision during `JSON.parse`, silently. There are
  three states for a missing value (`undefined`, `null`, and absent), and `JSON.stringify`
  drops `undefined` keys entirely, so a round-trip is not the identity function. `Date` is a
  timestamp with no timezone and no date-only variant, and its parsing rules are inconsistent
  by specification: `new Date("2026-08-29")` is UTC, `new Date("2026-08-29T00:00:00")` is local.
  TypeScript's types are erased at runtime, so `as User` validates nothing at all.

- **Python.** `None` is the only empty value, so *absent* and *null* collapse into one thing,
  and telling them apart requires sentinel objects. `datetime` is a subclass of `date`, so
  `isinstance(dt, date)` is `True` for a datetime and a whole class of checks pass when they
  shouldn't. Naive and aware datetimes are both `datetime`, they cannot be compared without a
  `TypeError`, and serializing a naive one silently discards the offset. `int` is arbitrary
  precision, so Python will happily hand a peer a number that peer cannot hold.

- **Java.** The same absent/null collapse, plus primitives that cannot be null at all: an
  optional `int` has to be boxed to `Integer`, and Jackson needs explicit configuration before
  absent and null behave differently. `java.time` and the legacy `java.util.Date` coexist, and
  `LocalDate`, `Instant` and `ZonedDateTime` are not interchangeable. There are no unsigned
  integer types, so `u64` has nowhere to land.

- **Rust** is the outlier, and that is the whole point. `Option<T>` makes absence explicit in
  the type system, `Option<Option<T>>` distinguishes absent from null without a sentinel,
  enums are real sum types, there is no `null`, and integer width and signedness are part of
  the type. **Rust is the only one of these languages that can represent every distinction the
  others lose.** That is why the contract belongs there, not because Rust is fast.

## The solution: the schema is a file, not code

The contract is a `.seam` file. It lives in your repo, it shows up in diffs, it gets reviewed
in pull requests. `seam-core` loads and compiles it **at runtime**, so adding a field never
means recompiling a native extension or republishing wheels.

```
schema User {
  id:          u64
  name:        String            @min_len(3) @max_len(64)
  age:         u32               @range(18..=120)
  plan:        enum { free, pro, enterprise }
  tags:        [String]          @max_items(10)

  nickname:    String?           // present, may be null
  bio:         optional String   // may be absent
  avatar:      optional String?  // may be absent OR null
}
```

```python
from seam import Schema, ValidationError

User = Schema.load("contracts/user.seam")     # runtime: no Rust toolchain, no build step

try:
    user = User.validate(payload)
except ValidationError as e:
    print(e.path, e.code, e.message)          # ["age"], "out_of_range", "18 <= age <= 120"
```

```typescript
import { Schema } from "seam";

const User = await Schema.load("contracts/user.seam");
const user = User.validate(payload);          // throws SeamValidationError
```

Same file. Same engine. Same verdict.

### Absence and nullability are orthogonal

Every tool in this space conflates "the key wasn't sent" with "the key was sent as null."
Seam treats them as two independent axes, because at an API boundary they mean different
things: one is *don't touch this field*, the other is *clear this field*. PATCH endpoints have
been getting this wrong for a decade.

| `.seam` | Rust | Python | TypeScript | Java |
|---|---|---|---|---|
| `String` | `String` | `str` | `string` | `String` |
| `String?` | `Option<String>` | `str \| None` | `string \| null` | `@Nullable String` |
| `optional String` | `Option<String>` | `str \| Absent` | `string \| undefined` | `Optional<String>` |
| `optional String?` | `Option<Option<String>>` | `str \| None \| Absent` | `string \| null \| undefined` | `JsonNullable<String>` |

### Dates and integers are pinned, not inferred

| `.seam` | Rust | Python | TypeScript | Java |
|---|---|---|---|---|
| `Date` | `NaiveDate` | `datetime.date` | ISO `string`, **not** `Date` | `LocalDate` |
| `DateTime` | `DateTime<Utc>` | aware `datetime` | `Date` | `Instant` |
| `u64` | `u64` | `int` | `bigint`, **not** `number` | `long`, range-checked |
| `i32` | `i32` | `int`, range-checked | `number` | `int` |

Two opinionated calls fall out of this, and Seam makes them explicitly:

- **A naive datetime is a validation error, not a guess.** No implicit local-time assumption,
  ever. A `DateTime` carries an offset or it does not validate.
- **`Date` never becomes a JavaScript `Date`.** A calendar date is not an instant, and forcing
  it through `Date` is the origin of every off-by-one-day bug in the table above.

## This is the actual product

A Rust core with language bindings is not novel; `jsonschema-rs` already ships one. What Seam
commits to is the part everyone else leaves undocumented:

1. **A normative mapping specification.** Every row in the tables above is a written rule with
   a rationale, not an implementation detail that might change in a patch release.
2. **A shared conformance suite.** A directory of plain-text cases (input, expected verdict,
   expected error) that runs identically against *every* binding in CI. A binding that
   disagrees with the suite fails the build.

```json
{
  "case": "u64 above MAX_SAFE_INTEGER survives the boundary",
  "schema": "user.seam",
  "input": { "id": 9007199254740993 },
  "expect": { "valid": true, "normalized": { "id": "9007199254740993" } }
}
```

"No drift" stops being a slogan the moment it becomes a test that can fail. That suite is the
real deliverable, and it is what survives even if someone reimplements the engine.

## Fast: not yet, and here are the numbers

Seam compiles a schema once into a validator tree and reuses it across every call, so parsing
and rule resolution happen at load time rather than per payload. That part works.

**The rest does not, and the benchmarks say so.** Validating a payload currently copies it into
an intermediate representation before checking anything, and that copy costs roughly **10x
msgspec and 2x pydantic v2**, rising to 18x on array-heavy payloads. Full methodology, hardware,
library versions and a script that reproduces it are in [`benchmarks/`](benchmarks/).

Removing that copy is the next piece of work. Until it lands, treat speed as a design goal that
has not been delivered, and read the numbers rather than this paragraph.

The 5-50x figure often quoted from `pydantic-core` was measured against pure-Python Pydantic v1
and does not transfer to a different engine with a different FFI shape. Every number Seam
publishes will come with the script that produced it.

## Safe

- **Memory safety** from the core outward: no hand-written C, no manual buffer arithmetic.
- **No panics cross the FFI boundary.** A failure surfaces as `ValidationError` in Python and
  `SeamValidationError` in JS. A panic reaching a foreign runtime is treated as a Seam bug.
- **Schemas are declarative data, not executable code.** Loading a `.seam` file cannot run
  anything. There is no expression language and no eval, so a schema from an untrusted source
  is a parsing problem rather than a sandboxing problem.
- **Hostile input is bounded.** Configurable limits on nesting depth, collection length, and
  total input size, enforced in the core so every binding inherits them.
- **Stable error contract.** `path`, `code`, and `message` are public API. `code` is a stable
  machine-readable identifier and changes only on a major version.

## Scope

Seam is deliberately narrow. It is **not** a replacement for Protobuf, OpenAPI, or serde. It
validates data at a boundary and states precisely what that data means. The goal is to nail the
cases that cause the most cross-language bugs, not to cover every schema feature.

**Phase 1: Core + Python** *(current)*
Strings, integers with explicit width and signedness, `f64`, `bool`, `Date`, `DateTime`, enums
with fixed value sets, arrays, arbitrarily nested objects, and the full four-state absent/null
matrix. Idiomatic errors through PyO3. The conformance suite is established here.

**Phase 2: Node.js.** The same surface via `napi-rs`, `bigint` for 64-bit integers, `Error`
subclasses rather than generic throws, and the conformance suite green.

**Phase 3: WebAssembly.** Browser-side validation via `wasm-bindgen`, so a frontend enforces
the same contract as the service it calls.

**Phase 4: Expanded types.** Unions and tagged variants, custom validators, string formats,
cross-field constraints, and date/time edge cases beyond the basics.

**Phase 5: JVM.** Via the Panama FFM API (Java 22+), not JNI. This is honestly a large lift:
off-heap memory management, its own platform matrix, and a Java ecosystem that expects a pure
jar. It is on the roadmap because the Java pain described above is real, but it is a later phase
and not a near-term promise.

**Out of scope:** RPC and transport, general-purpose serialization, and code generation for
languages with no viable native FFI story.

## Architecture

A Cargo workspace. The engine lives in one crate; every binding is a thin translation layer.

```
seam/
├── Cargo.toml            # workspace root
├── spec/                 # normative mapping specification
├── conformance/          # shared test cases, every binding runs these
├── seam-core/            # the engine: .seam parser, compiler, validator, errors
├── seam-macros/          # optional #[derive(Schema)] for Rust-first users
├── seam-py/              # PyO3 binding
├── seam-js/              # napi-rs binding      (phase 2)
├── seam-wasm/            # wasm-bindgen binding (phase 3)
└── seam-jvm/             # Panama binding       (phase 5)
```

```
                        contracts/user.seam
                                 │
                    ┌────────────▼─────────────┐
                    │        seam-core         │
                    │  parse → compile → run   │
                    │  validation logic, once  │
                    └────────────┬─────────────┘
                                 │
         ┌───────────┬───────────┴───────────┬───────────┐
         │           │                       │           │
    ┌────▼────┐ ┌────▼────┐           ┌──────▼──┐ ┌──────▼──┐
    │ seam-py │ │ seam-js │           │seam-wasm│ │seam-jvm │
    │  PyO3   │ │ napi-rs │           │ bindgen │ │ Panama  │
    └────┬────┘ └────┬────┘           └────┬────┘ └────┬────┘
         │           │                     │           │
    pip install  npm install          import from    Maven
         │           │                     │           │
         └───────────┴──────────┬──────────┴───────────┘
                                │
                    conformance/ runs against ALL
```

**Golden rule:** a binding translates types and errors. It never re-implements a rule. If
`seam-py` starts growing validation logic of its own, that is a leak in `seam-core` to be fixed
there.

`seam-macros` is a separate crate because Rust requires proc macros to live in their own
`proc-macro = true` crate. It is optional sugar: it emits the same schema representation the
`.seam` parser produces, so Rust-first users get one source of truth without making Rust a
prerequisite for everyone else.

## Prior art, honestly

Seam is not the first tool near this problem, and the differences are worth stating.

- **JSON Schema** is genuinely portable, and validators exist everywhere, but the specification
  says little about how a validated document maps into each language's types. Two conformant
  validators can accept the same document and hand their host wildly different values. That gap
  is exactly where Seam lives.
- **protovalidate** (Buf) is the closest prior art: one schema, shared rules, many languages. It
  requires adopting Protobuf as your wire format and expresses constraints in CEL, an embedded
  expression language. Seam has no wire-format opinion and no expression language.
- **CUE** is more powerful and more general. It is also a language to learn, and its center of
  gravity is configuration rather than runtime boundary validation.
- **msgspec / pydantic v2 / Zod** are excellent, and faster than Seam will be within a single
  language. They are single-language by design; they are the benchmark bar, not the target.

If you work in one language, use the native tool. Seam is for the seam between them.

## Status

Early development. Phase 1 in progress. Not yet published to crates.io or PyPI. The mapping
specification and the conformance suite are being written alongside the engine, not after it.

## License

MIT OR Apache-2.0, at your option. This is the standard dual license of the Rust ecosystem.

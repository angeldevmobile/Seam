# Seam

**One schema. Every language. No drift.**

Seam is a cross-language data contract: you write the schema once in a `.seam` file, and
Python, Node and the browser (with the JVM to come) all validate against the *same compiled
engine*, with the same rules, the same errors, and the same answers on the cases that usually
break.

There is no generated mirror type to keep in sync. There is no second validator to drift.

| language | package | status |
|---|---|---|
| **Rust** | [`seam-core`](https://crates.io/crates/seam-core) | the engine itself |
| **Python** | [`seam-schema`](https://pypi.org/project/seam-schema/) | 3.9+, one `abi3` wheel per platform, generated `TypedDict`s |
| **Node** | [`seam-schema`](https://www.npmjs.com/package/seam-schema) | 20+, native module, `bigint` for 64-bit, generated TypeScript |
| **Browser** | [`seam-schema-wasm`](https://www.npmjs.com/package/seam-schema-wasm) | WebAssembly, 55 KiB brotli, bytes only |
| **JVM** | none yet | planned, via Panama |

---

## See it break, then see it not

[**seam-demo**](https://github.com/angeldevmobile/seam-demo) is a Python service that sends an
order and a Node consumer that reads it. The order ID arrives **off by 83**, nothing throws, and
every system downstream now disagrees with the database, because `id` is a snowflake and
`JSON.parse` cannot hold an integer past 2^53.

```
  JSON.parse                              seam-schema
  ─────────────────────────────           ─────────────────────────────
  Python sent   1847559241284567891       Python sent   1847559241284567891
  Node received 1847559241284567800       Node received 1847559241284567891
  off by        83                        off by        0
  typeof id     number                    typeof id     bigint
  threw         nothing                   threw         nothing
```

The same `.seam` file drives all of it: the Python producer, the Node and TypeScript consumers,
and a browser page running the engine as WebAssembly. Nobody restates the contract, which is the
only reason the four cannot drift apart.

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
  contact:     String            @format(email)
  plan:        enum { free, pro, enterprise }
  tags:        [String]          @max_items(10)

  nickname:    String?           // present, may be null
  bio:         optional String   // may be absent
  avatar:      optional String?  // may be absent OR null
}
```

```python
from contracts.user_types import validate_user   # generated by `seam typegen`

user = validate_user(payload)   # a dict, or raw JSON bytes; it does not matter which
```

Seam parses the JSON itself. That is not a convenience: `JSON.parse` would already have
corrupted the `u64` above before any validator could see it, and no amount of checking
afterwards brings the bits back. A caller never has to know that.

```python
from seam_schema import ValidationError

try:
    user = validate_user(payload)
except ValidationError as e:
    print(e.path, e.code, e.message)          # "age", "out_of_range", "18 <= age <= 120"
```

```javascript
const { Schema } = require("seam-schema");

const User = Schema.load("contracts/user.seam").validator("User");
const user = User.validate(rawRequestBody);   // throws SeamValidationError
user.id;                                      // 9007199254740993n, a bigint, exact
```

```typescript
import { Schema } from "seam-schema";
import type { UserTypes } from "./contracts/user.types";   // generated by `seam typegen`

const schema = Schema.load<UserTypes>("contracts/user.seam");
const user = schema.validator("User").validate(rawRequestBody);

user.id;      // bigint
user.bio;     // string | undefined -- absent is a different state from null
```

```javascript
// In the browser. Same engine, compiled to WebAssembly.
import { Schema } from "seam-schema-wasm";

const schema = await Schema.parse(await (await fetch("/contracts/user.seam")).text());
const user = schema.validator("User").validate(await res.arrayBuffer());   // not res.json()
```

The browser build takes bytes and **refuses an already-parsed object**. By the time
`JSON.parse` has run the `u64` above is already the wrong number, so accepting one would offer,
in the runtime where the problem is worst, the exact path Seam exists to avoid.

Same file. Same engine. Same verdict. The conformance suite runs from Rust, Python, Node and
WebAssembly against those same case files, in CI, so "same verdict" is a test rather than a
promise.

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

### Tagged unions, with the tag written down

```
union Event @tag("type") {
  created: Created
  deleted: Deleted
}
```

`@tag` is mandatory. There is no default and no inference from the variants, for the same
reason a naive datetime is an error rather than a guess: a union that decided for itself which
field discriminates would be guessing what the data means.

The tag belongs to the union, so no variant may declare it: that would be two sources of truth
for one value. It is still carried through to the validated value, it is never reported as an
unknown field, and a variant is not a level of nesting: an issue inside the chosen variant is
`latest.amount`, never `latest.created.amount`.

Both generators pin the tag to a literal, so a type checker narrows on it:

```typescript
if (event.type === "created") {
  event.amount;        // bigint, and the compiler knows which variant this is
}
```

```python
if event["type"] == "created":
    event["amount"]    # int, and mypy knows it too
```

A tag naming no variant is `unknown_variant`, and it is the *only* issue reported: with no
variant chosen there is no shape to check the rest against, and picking one anyway would
produce a list of errors about a payload nobody claimed to send.

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

## Installing

```bash
cargo add seam-core            # Rust: the engine
pip install seam-schema        # Python 3.9+
npm install seam-schema        # Node 20+
npm install seam-schema-wasm   # browsers
```

The native packages ship binaries for Linux x64, macOS ARM and Windows x64. Linux means glibc
2.17 and up, which is `manylinux2014`: Ubuntu 14.04 onwards, Debian 8 onwards, RHEL 7
onwards. Not musl, so not Alpine. On any other platform `pip install` and `npm install` fail rather than
falling back, because there is no pure implementation to fall back to; `seam-schema-wasm` runs
anywhere and is the answer there.

On npm the binary lives in a per-platform package, `seam-schema-linux-x64-gnu` and its two
siblings, which `seam-schema` pulls in as an optional dependency. That is what keeps the
package you install at 13 KB instead of 1 MB of binaries for machines you are not on. Two
consequences worth knowing before they bite:

- **`npm install --no-optional` produces a package that cannot load.** So does a private
  registry or an allowlist that mirrors `seam-schema` without its platform packages. The error
  says npm has a bug and suggests deleting `package-lock.json`; when the cause is either of
  these, it does not.
- The lockfile records all three platform packages. That is npm working as intended, not
  bloat: it is what lets a Linux CI and a Mac laptop install from one lockfile.

From a checkout, if you would rather build it. Each binding needs its own toolchain:

```bash
git clone https://github.com/angeldevmobile/Seam
cd Seam

cargo test                                  # the engine

pip install ./seam-py && pytest seam-py/tests

cd seam-js   && npm install && npm run build && npm test

rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.127
cd seam-wasm && npm install && npm run build && npm test
```

The Python and Node packages both install a `seam` command, which takes the same subcommands and
flags in either ecosystem. `seam typegen contracts/user.seam` writes the generated types beside
the schema; `seam typegen --check` fails when they have fallen behind it, which is what belongs
in CI.

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
  "schema": "user",
  "type": "User",
  "cases": [
    {
      "name": "u64 at the JS MAX_SAFE_INTEGER boundary is exact",
      "note": "JSON.parse corrupts this to ...992. A conformant runner must not.",
      "input": { "id": 9007199254740993, "name": "Gabriel", "plan": "pro", "nickname": null },
      "input_raw": "{\"id\": 9007199254740993, \"name\": \"Gabriel\", \"plan\": \"pro\", \"nickname\": null}",
      "expect": "valid"
    }
  ]
}
```

`expect` is `"valid"` or an object listing `issues`, each asserting `path` and `code` and never
`message`. `input_raw` carries the same payload as JSON *text*, because a runner in JavaScript
would otherwise corrupt that integer with `JSON.parse` while merely loading the case file.

"No drift" stops being a slogan the moment it becomes a test that can fail. That suite is the
real deliverable, and it is what survives even if someone reimplements the engine.

Today it is 95 cases over four schemas, asserting **every one** of the spec's 21 error codes,
run by four runners on three operating systems.

Two of those runners feed the binding host objects and two feed it bytes, and that split is what
found the two places they disagreed. A limit was a parse error on one path and a coded issue on
the other, so a service handling `size_exceeded` would have missed it on the path this project
recommends. An undeclared type name was `unknown_type` in Python and a generic failure in Node.
Both are fixed; neither would have been visible from a suite that fed every binding the same
way.

## Fast, and here is what that means

Measured from raw JSON bytes, which is what a service actually holds when a request lands:

| | seam | msgspec | pydantic v2 |
|---|---:|---:|---:|
| flat, 6 fields | 2 120 ns | **564** | 2 219 |
| nested + a date | 4 353 ns | **843** | 3 308 |
| array of 100 strings | 13 354 ns | **4 471** | 6 828 |

That is one run, and one run of the flat row is not evidence. Across eight, each pair measured in
the same session, **a flat payload costs between 0.93x and 1.32x pydantic v2, median 1.10x**,
with Seam ahead in two of them. On a flat payload the two are inside each other's noise, and the
honest statement is that neither is reliably faster. Nested is about 1.2x and the array row about
2x, which is where a schema read at runtime costs the most.

msgspec stays several times ahead in every scenario, and that is structural rather than fixable:
its fields are slots in a layout fixed at compile time, while a Seam schema is a file read at
runtime. That is the price of the schema being portable, and it is the whole point rather than a
defect.

Four things earn those numbers, and each was measured on its own before it was kept:

- **The schema compiles once.** Binding a validator resolves the type, the limits and the host
  classes, so none of it is paid per call.
- **Nothing is copied to be validated.** The engine is generic over an input trait, so it reads
  the host's objects, or the JSON buffer, where they already are.
- **Seam parses the JSON.** For correctness, as the next section explains, but it also removes
  a whole pass and makes Seam faster from bytes than from a dict.
- **An error costs nothing until it is read.** Rejecting builds one object; the paths, the issue
  list and the message appear only if something asks.

Full methodology, hardware, library versions, the things that were tried and removed for being
unmeasurable, and scripts that reproduce all of it are in [`benchmarks/`](benchmarks/). Read the
numbers rather than this paragraph.

The 5-50x figure often quoted from `pydantic-core` was measured against pure-Python Pydantic v1
and does not transfer to a different engine with a different FFI shape. Every number Seam
publishes comes with the script that produced it.

### Why Seam parses the JSON itself

Look again at the table at the top of this README. `JSON.parse` turns `9007199254740993` into
`...992` before any validator is called, and no amount of checking afterwards brings the bits
back. A library handed already-parsed host objects cannot keep the promise this project is
built on.

Python was generous about this: `json.loads` uses arbitrary-precision integers, so the dict path
was already correct. That was the language rather than a design decision, and JavaScript is not
generous. So Seam reads the bytes itself, applying its own rules about precision while it does.

It parses in order to validate. There is no encoder, which is what keeps the line below about
serialization true.

## Safe

- **Memory safety** from the core outward: no hand-written C, no manual buffer arithmetic.
- **The parsers are fuzzed.** Both readers are written by hand, so a deterministic, seeded fuzzer
  runs against them on every push: two million mutated inputs, asserting only that nothing
  unwinds. Rejecting an input is a correct answer and accepting one is a correct answer; panicking
  is not, because it crosses into a runtime that cannot catch it. A failure prints the seed that
  reproduces it, and inputs that once broke something stay in a regression list that runs in
  milliseconds.
- **No panics cross the FFI boundary.** A failure surfaces as `ValidationError` in Python and
  `SeamValidationError` in JS. A panic reaching a foreign runtime is treated as a Seam bug.
- **Schemas are declarative data, not executable code.** Loading a `.seam` file cannot run
  anything. There is no expression language and no eval, so a schema from an untrusted source
  is a parsing problem rather than a sandboxing problem.
- **Hostile input is bounded.** Configurable limits on nesting depth, collection length, and
  total input size, enforced in the core so every binding inherits them. Nesting depth has a
  hard ceiling no binding can raise: it is the only limit whose breach is a dead process rather
  than an error, because both the reader and the validator recurse.
- **Stable error contract.** `path`, `code`, and `message` are public API. `code` is a stable
  machine-readable identifier and changes only on a major version.

[SECURITY.md](SECURITY.md) says which of these are vulnerabilities rather than bugs, where the
known limits are, and how to report one privately.

## Scope

Seam is deliberately narrow. It is **not** a replacement for Protobuf, OpenAPI, or serde. It
validates data at a boundary and states precisely what that data means. The goal is to nail the
cases that cause the most cross-language bugs, not to cover every schema feature.

**Phase 1: Core + Python.** *Done.*
Strings, integers with explicit width and signedness, `f64`, `bool`, `Date`, `DateTime`, enums
with fixed value sets, arrays with their own element nullability, arbitrarily nested objects,
and the full four-state absent/null matrix. A `.seam` parser, a JSON parser, idiomatic errors
through PyO3, generated `TypedDict`s, and the conformance suite running from both Rust and
Python against the same files.

**Phase 2: Node.js.** *Done.* The same surface via `napi-rs`, `bigint` for 64-bit integers, a
real `Error` subclass with a real stack, `undefined` read as absence, generated TypeScript
interfaces, and the conformance suite running from Node against the same files as Rust and
Python. This is the phase that made the thesis demonstrable rather than merely argued.

**Phase 3: WebAssembly.** *Done.* Browser-side validation via `wasm-bindgen`, so a frontend
enforces the same contract as the service it calls: 56 KiB brotli, `bigint` for 64-bit integers,
and the same conformance suite as the other three. It takes bytes and refuses an already-parsed
object, because `JSON.parse` has already corrupted the value by then.

**Phase 4: Expanded types.** Tagged unions are *done*: a `union` with a mandatory `@tag`,
discriminated unions in TypeScript and tag-pinned `TypedDict`s in Python, both narrowing on the
tag. String formats are done too: `@format(uuid)`, `email`, `hostname`, `ipv4`, `ipv6`, as a closed
set of names rather than a regex, so no pattern can burn unbounded time and the engine keeps its
zero dependencies. Still open: date/time edge cases beyond the basics.

*Custom validators are no longer on this list, and neither are general cross-field constraints.*
Both need an expression language, and the "Safe" section above promises there is none: a schema
is data, so loading one from an untrusted source is a parsing problem rather than a sandboxing
problem. That promise is worth more than the feature. What a caller actually wants from a custom
validator is business logic, which belongs in the host, after Seam has guaranteed the shape it
runs on. Where a cross-field rule is common enough to earn a name it can get one. A closed
vocabulary like `@requires(other)` states the relationship without evaluating anything, but
never an expression.

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
├── benchmarks/           # numbers, methodology, and the scripts behind them
├── seam-core/            # the engine
│   ├── input.rs          #   what the validator needs from a payload
│   ├── json.rs           #   JSON read in place, precision rules applied there
│   ├── parser.rs         #   the .seam front end
│   ├── schema.rs         #   the compiled type model
│   └── validate.rs       #   one walk, generic over the input
├── seam-py/              # PyO3 binding, abi3 wheels for 3.9+
├── seam-js/              # napi-rs binding, bigint for 64-bit integers
├── seam-wasm/            # wasm-bindgen binding, browser, bytes only
└── seam-jvm/             # Panama binding       (phase 5, not built yet)
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

`seam-core` has **no dependencies**, and that is deliberate rather than incidental. It is loaded
into Python, Node and the browser, so every transitive dependency would be one those three
runtimes inherit, on a crate whose whole job is to be trustworthy about untrusted input. It is
also why the JSON parser is written by hand: a general-purpose one would rebuild the
intermediate tree the input trait exists to avoid, and would weigh on a WebAssembly build.

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

Early development, published at `0.1.3`. Phases 1 through 3 are done, and phase 4 is partly
done: tagged unions and string formats exist, date/time edge cases do not.

`0.x` means the contract can still move, which is what [`spec/`](spec/) says of itself: nothing
is frozen until 1.0. Within `0.x`, a renamed or removed error code is breaking and bumps the
minor. [CHANGELOG.md](CHANGELOG.md) records what has moved so far, including the two releases
that can break an install that was working.

**What works today.** A `.seam` file parses, compiles and validates. Python gets an `abi3` wheel
covering 3.9 and up and generated `TypedDict`s; Node gets a native module, `bigint` where a
`number` would lie, and generated TypeScript interfaces; the browser gets the same engine as
WebAssembly at 55 KiB brotli. All three take raw JSON in one call (Python and Node also take a
dict or object), normalise values on the way out, and build nothing for an error until something
reads it. The conformance suite runs from Rust, Python, Node and WebAssembly against the same 95
cases, covering every one of the spec's 21 error codes, in CI, on three operating systems. Both
hand-written parsers are fuzzed on every push.

**What is honest about it.** Framework integration does not exist, and neither do custom
validators or general cross-field constraints: those two are not "not yet", they are declined,
because they need an expression language and the security model promises there is none. There is
no `@pattern` either, for the same reason. If you work in a single language today, pydantic or
msgspec is the better tool and this README says so plainly further up. The reason to reach for
Seam is the second language, and the measurement of whether it delivers is the conformance suite
rather than anything argued here.

The mapping specification in [`spec/`](spec/) and the suite in [`conformance/`](conformance/)
are written alongside the engine rather than after it, and the numbers in
[`benchmarks/`](benchmarks/) are published with the scripts that produced them, including the
runs that came out badly.

## License

MIT OR Apache-2.0, at your option. This is the standard dual license of the Rust ecosystem.

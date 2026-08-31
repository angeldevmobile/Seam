# Conformance suite

The cases every binding must agree on. A binding that disagrees fails the build.

This is what turns "no drift" from a claim into something that can fail.

## Layout

```
conformance/
├── schemas/       .seam files the cases refer to
└── cases/         one file per area of the mapping spec
```

## Case format

```json
{
  "schema": "user",
  "type": "User",
  "cases": [
    {
      "name": "a u64 above MAX_SAFE_INTEGER survives",
      "input": { "id": 9007199254740993, "name": "Gabriel", "plan": "pro", "nickname": null },
      "expect": "valid"
    },
    {
      "name": "a naive datetime is rejected",
      "input": { "when": "2026-08-29T14:30:00" },
      "expect": { "issues": [{ "path": "when", "code": "missing_timezone" }] }
    }
  ]
}
```

`expect` is either `"valid"` or an object with `issues`. Each issue asserts
`path` and `code` only — `message` is not stable and is never asserted.

Issue order is significant: the spec fixes it as declaration order, then unknown
keys sorted, depth first.

## `input_raw`, and why it has to exist

A case may carry `input_raw`: the same payload as a **string of JSON text**.

It is there because a runner has to read the case file with its own language's
tools, and in JavaScript that means `JSON.parse`, which corrupts any integer
past 2^53 while merely loading the test. A runner in such a language uses
`input_raw` when it is present, so the bytes reach the binding exactly as
written. Runners in languages with exact integers may ignore it.

Only the cases that turn on precision carry it. That it is needed at all is the
point the suite is making.

## Writing input in JSON

JSON cannot express everything the cases need, so two conventions:

- Integers are written as JSON numbers. A runner **must** parse them without
  going through a double — that is the property being tested. In JavaScript this
  means a JSON parser with `bigint` support, not `JSON.parse`.
- An absent key is written by omitting it. An explicit null is `null`. These are
  different cases and both appear.

## Running

```bash
cargo test --test conformance
```

The reference runner is `seam-core/tests/conformance.rs`. It runs in CI, so a
change that breaks a case fails the build.

**Lowering is part of what is under test, not a detail of the harness.** A
binding that corrupts an integer while converting its host's values is
non-conformant even if the validator it calls is perfect. That is why
`integer_too_wide` is produced during lowering rather than by the core: the
core's integer type cannot hold a 65-bit value, so a binding has to catch it
first. Every binding's runner has to do the same.

The runner has a self-test (`the_harness_detects_a_wrong_expectation`) that
feeds it a deliberately wrong expectation. A harness that cannot fail proves
nothing.

## `limits`, and `codes`

A case may set the engine's limits for itself:

```json
{
  "name": "nesting past max_depth is depth_exceeded",
  "limits": { "max_depth": 2 },
  "expect": { "codes": ["depth_exceeded"] }
}
```

The keys are spelled as the spec spells them. A binding whose own API
camelCases them translates in its runner, which is the only place the two
spellings meet.

`expect.codes` asserts **which codes appeared**, not where or how many times,
and exists for limits alone. A limit is checked while the document is being
read, before the value it belongs to exists — that is what makes it a bound on
hostile input rather than a report about one. A binding handed bytes therefore
stops at the first breach and has no path; a binding handed host objects
finishes the walk and reports each at its own path. The code is what both must
agree on, and the code is the stable API. Every other case uses `issues` and
asserts the path exactly.

## Cases that depend on the host

Two rules in the mapping spec are about what a *host* can hold rather than
about what the data says, and a case can express that:

```json
{
  "name": "an integer past 2^53 is refused where the host cannot hold it",
  "host_value": true,
  "input": { "id": 9007199254740993 },
  "expect": "valid",
  "expect_inexact_integers": {
    "issues": [{ "path": "id", "code": "unsafe_integer" }]
  }
}
```

`host_value` means feed the binding the runtime's own values rather than bytes.
A binding with no such path — `seam-wasm` takes bytes only, on purpose — skips
these and **says it skipped**, because a case that passes by not running is
worse than one that fails.

`expect_inexact_integers` is the verdict for a host whose integers are not
exact past 2^53. A runner in such a language takes it; the others take
`expect`. The JavaScript runner is its own demonstration: it loads the case
file with `JSON.parse`, which corrupts that id before the runner has done
anything, which is precisely the condition `unsafe_integer` names.

This is the only place a case has two verdicts, and it is the only rule in the
spec whose answer depends on the host rather than on the payload.

## Coverage

78 cases over three schemas, asserting **all 20** codes in the mapping spec.

Getting the last two took finding two real defects rather than writing two
cases, which is what the suite is for:

- `unknown_type` was reported by Python with its code and by Node and wasm as a
  generic failure. It was also reported at the type's name, where `path` is a
  route through the payload and a type name is not a key of anything; it is at
  the root now, as the engine always had it.
- `unsafe_integer` had no case at all, and the rule was checked only inside one
  binding's own tests, unattached to the spec that requires it.

## Runners

Four runners share these files today: `seam-core/tests/conformance.rs`,
`seam-py/tests/test_conformance.py`, `seam-js/__test__/conformance.test.mjs`
and `seam-wasm/__test__/conformance.test.mjs`. They run in CI on Linux, macOS
and Windows.

The first two feed the binding host objects; the last two feed it bytes. That
split is deliberate, and it is what caught both places the two paths disagreed:
limits, and an undeclared type name. Agreement between them is what "no
drift" means, and it is checked on every push rather than asserted here.

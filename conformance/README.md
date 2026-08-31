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

## What this suite does not cover

68 cases over three schemas, asserting 16 of the 20 codes in the mapping spec.
The four it does not reach, and why, because an uncovered code should be a
known gap rather than an oversight:

- **`unsafe_integer`** cannot be produced from bytes. It means *the host's own
  value is already the wrong number*, which is a condition of a JavaScript
  `number` past 2^53 and of nothing the suite can write as JSON text. It is
  tested in `seam-js/__test__/binding.test.mjs`, where it belongs.
- **`depth_exceeded`** and **`size_exceeded`** need limits below the defaults,
  and a case file cannot yet set them. Adding a `limits` key to the document
  and honouring it in all three runners would close both.
- **`unknown_type`** is unreachable by construction: the `.seam` parser rejects
  a dangling reference at parse time, and every binding rejects an undeclared
  type name when the validator is bound, before validation begins.

## Runners

Three runners share these files today: `seam-core/tests/conformance.rs`,
`seam-py/tests/test_conformance.py` and `seam-js/__test__/conformance.test.mjs`.
All three run in CI on Linux, macOS and Windows. Agreement between them is what
"no drift" means, and it is checked on every push rather than asserted here.

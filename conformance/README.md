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

Remaining: a runner in each binding, sharing these files.

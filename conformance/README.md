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

Not runnable yet: the cases refer to `.seam` files and the parser is not
implemented. Until it is, the same assertions live as unit tests in
`seam-core/src/validate.rs`, and this directory is the specification of what the
runner will check.

Order of work: parser → runner in `seam-core` → runner in each binding.

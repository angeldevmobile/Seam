# seam-core

The validation engine behind [Seam](https://github.com/angeldevmobile/Seam):
one schema, every language, no drift.

You write a contract once, as a `.seam` file, and Python, Node and the browser
validate against **this same compiled engine**, with the same rules, the same
errors and the same answers on the cases that usually differ between languages.

```rust
use seam_core::{parse, validate, value::Value, Limits};
use std::collections::BTreeMap;

let schema = parse("schema Signup { when: DateTime }")?;

let mut payload = BTreeMap::new();
payload.insert("when".into(), Value::String("2026-08-29T14:30:00".into()));

let err = validate(&schema, "Signup", &Value::Object(payload), Limits::DEFAULT)
    .unwrap_err();
assert_eq!(err.issues[0].code.as_str(), "missing_timezone");
# Ok::<(), seam_core::parser::ParseError>(())
```

A naive datetime is an error, not a guess. "Local" is different on each side of
a boundary, so assuming one produces a value that is wrong somewhere.

## What it is for

The bugs this exists to stop do not happen inside a language. They happen at the
seam between two, because each quietly disagrees about what the data means:

- `9007199254740993` becomes `...992` in JavaScript, silently, during
  `JSON.parse`.
- *absent* and *null* collapse into one thing in Python and Java, so "the user
  cleared their nickname" is indistinguishable from "the client did not send
  the field".
- A calendar date pushed through a JavaScript `Date` is a day earlier in
  UTC-5.

This crate holds the rules that refuse all three, so that every binding
inherits the same answer instead of deriving its own.

## No dependencies, on purpose

The dependency list is empty and meant to stay that way. This crate is loaded
into Python, Node and a browser bundle, so anything it pulls in, those hosts
inherit, and it is a library whose entire job is to be trustworthy about
untrusted input. Date handling and string formats are written here rather than
pulled in for the same reason.

## What it promises

- **Absence and nullability are separate axes.** `optional T` and `T?` are
  different types, because at a boundary they mean different things.
- **Integer width is part of the type**, so a range check means the same thing
  in every language.
- **No integer passes through a float.** The JSON reader keeps the literal,
  which is the only way `u64` survives.
- **Hostile input is bounded**, with a hard ceiling on nesting depth that no
  caller can raise: both readers recurse, and a stack overflow kills a process
  rather than raising something catchable.
- **Nothing panics.** Both parsers are fuzzed on every push with millions of
  mutated inputs, asserting only that nothing unwinds.

The normative rules live in [`spec/mapping.md`](https://github.com/angeldevmobile/Seam/blob/main/spec/mapping.md),
and every one of them has a case in the shared conformance suite that four
runners execute against the same files.

## Bindings

| language | package |
|---|---|
| Python | `seam-schema` |
| Node | `seam-schema` |
| Browser | `seam-schema-wasm` |

## License

MIT OR Apache-2.0, at your option.

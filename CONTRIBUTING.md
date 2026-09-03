# Contributing

The most useful thing you can send is a case the conformance suite does not
cover yet. Everything below is downstream of that.

## The one rule

**A binding translates types and errors. It never re-implements a rule.**

If `seam-py` or `seam-js` starts growing validation logic of its own, that is a
leak in `seam-core` to be fixed there. A rule that lives in two places is a rule
that will eventually disagree with itself, which is the exact failure this
project exists to prevent.

## Found a wrong verdict? Send a case.

Seam accepting a payload its schema forbids, or rejecting one it allows, is a
correctness bug. Open an issue, and if you can, write it as a conformance case.
A case is worth more than a description because it becomes the regression test.

Cases live in [`conformance/cases/`](conformance/cases), one file per area of
the mapping spec, and refer to schemas in
[`conformance/schemas/`](conformance/schemas):

```json
{
  "name": "a null element is rejected when the element type is not nullable",
  "input": { "tags": ["a", null] },
  "expect": { "issues": [{ "path": "tags[1]", "code": "null_not_allowed" }] }
}
```

`expect` is either `"valid"` or an object listing `issues`. An issue asserts
`path` and `code` only. `message` is for humans and is never asserted.

Two things to know before you write one:

1. **Issue order is significant.** The spec fixes it as declaration order, then
   unknown keys sorted, depth first.
2. **If your case turns on integer precision, add `input_raw`.** It is the same
   payload as a string of JSON text. A runner in JavaScript reads the case file
   with `JSON.parse`, which corrupts an integer past 2^53 while merely loading
   the test, so those runners use `input_raw` when it is present. That this is
   needed at all is the point the suite is making.

The suite runs from Rust, Python, Node and WebAssembly against the same files.
A case you add is a case all four have to agree on, which is the whole idea.

## Running things

Each binding needs its own toolchain. None of them needs the others.

```bash
cargo test                                  # the engine and the suite from Rust

pip install ./seam-py && pytest seam-py/tests

cd seam-js   && npm install && npm run build && npm test

rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.127
cd seam-wasm && npm install && npm run build && npm test
```

Before opening a pull request:

```bash
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
```

CI runs those on three operating systems, plus the fuzzer against both
hand-written parsers, plus a packaging smoke test that installs every package
from its packaged form outside the repository and uses it.

## Proposing a feature

Read [Scope](README.md#scope) first. Seam is deliberately narrow, and some
things are declined rather than pending:

- **Custom validators and general cross-field constraints.** These need an
  expression language, and the security model promises there is none: a schema
  is declarative data that cannot execute anything.
- **`@pattern`.** A backtracking regex lets a hostile schema or payload burn
  unbounded time, and a linear one would be the engine's first dependency and
  its weight in every host, including a browser bundle.

Those are not "not yet". Arguing them requires arguing the security model,
which is a fair thing to do, but do it directly rather than as a feature
request.

Anything that changes what a `.seam` file means also changes
[`spec/mapping.md`](spec/mapping.md) and needs conformance cases in the same
pull request. The spec is normative; the implementations follow it, not the
other way around.

## Reporting a vulnerability

Not through an issue. See [SECURITY.md](SECURITY.md).

## Commits

Say what was wrong, not what you did. `max_depth could be disabled until it
killed the process` is a better commit message than `fix limits`, because six
months later the first one still explains why the code looks the way it does.

## License

Contributions are licensed under MIT OR Apache-2.0, the same as the project.

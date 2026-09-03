# Security

## Reporting a vulnerability

Report it privately through GitHub's
[security advisory form](https://github.com/angeldevmobile/Seam/security/advisories/new).
Please do not open a public issue for a vulnerability.

Seam is maintained by one person, so a response is best effort rather than a
service level. You will get an acknowledgement that the report was read, and a
say in the timing of any disclosure.

## Supported versions

Seam is `0.x`. Fixes go to the latest released version only; there are no
maintenance branches. Upgrading within `0.x` is a patch or minor bump, and
[CHANGELOG.md](CHANGELOG.md) says what moved.

## What Seam is defending against

Seam's job is to read data it does not trust, so these are the properties it
holds itself to. A reproducible failure of any of them is a vulnerability, not
a bug report:

- **No panic crosses the FFI boundary.** A panic reaching Python or Node is a
  Seam bug: those runtimes cannot catch it, and the process is what pays. Both
  hand-written parsers are fuzzed on every push against two million mutated
  inputs, asserting only that nothing unwinds. Rejecting an input is a correct
  answer, accepting one is a correct answer, panicking is not.
- **Hostile input is bounded.** Limits on nesting depth, collection length,
  key count, string length and total input size are enforced in the core, so
  every binding inherits them whether or not the caller asks. An input that
  reaches unbounded time or memory inside those limits is a vulnerability.
- **Memory safety.** No hand-written C and no manual buffer arithmetic;
  `unsafe_code` is denied across the workspace.
- **A schema cannot execute anything.** `.seam` files are declarative data.
  There is no expression language and no eval, deliberately, so a schema from
  an untrusted source is a parsing problem rather than a sandboxing one. This
  is also why there is no `@pattern`: a backtracking regex would let a hostile
  schema or payload burn unbounded time.

## Known limits, by design

These are documented behaviour rather than vulnerabilities:

- **Nesting depth has a hard ceiling that no binding can raise.** It is the one
  limit whose breach is a dead process rather than an error, because both the
  reader and the validator recurse. Raising it is not configurable for that
  reason.
- **WebAssembly memory grows and is never returned to the host.** One oversized
  payload raises a tab's floor for the rest of its life, so
  `seam-schema-wasm` adds a `maxBytes` bound, checked before anything is copied
  into the module. It defaults to 8 MiB.
- **A wrong verdict is a correctness bug, not a vulnerability.** If Seam accepts
  a payload its schema forbids, or rejects one it allows, please open a normal
  issue with the case. That is what the conformance suite exists for, and a
  case that reproduces it is the most useful thing you can send.

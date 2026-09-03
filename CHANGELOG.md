# Changelog

Seam is `0.x`. The contract can still move, which is what [`spec/`](spec/) says
of itself: nothing is frozen until 1.0. Within `0.x`, a renamed or removed
error code is breaking and bumps the minor.

The four packages share one version number and are always released together,
even when a change touches only one of them. One number means one answer to
"which version of the contract is this".

## 0.1.3 — 2026-09-03

### Changed

- **The npm package no longer contains a binary.** `seam-schema` was 955 KB
  because it shipped all three platform binaries *and* declared the
  per-platform packages holding the same three. Every install downloaded its
  own binary twice, plus two it could never load. It is now 13 KB, and the
  binary arrives through `seam-schema-<platform>` as the optional dependency it
  should always have been.

  **This can break an install that worked before.** `npm install --no-optional`
  now produces a package that cannot load, and so does a private registry or an
  allowlist that mirrors `seam-schema` without its three platform packages. The
  error npm reports blames a bug in npm and suggests deleting
  `package-lock.json`; for either of those causes, it does not help. Nothing
  else changed: same API, same engine, same verdicts.

### Fixed

- The release gate validated a package npm never receives. `napi pre-publish`
  writes `optionalDependencies` into `package.json` and moves each binary into
  its per-platform package, and only then does `npm publish` run — but the
  packaging smoke test packed the checkout as it stood, with no optional
  dependencies and a binary sitting next to `index.js`. It passed on a shape
  nobody installs, which is the same blind spot that let 0.1.0 out. It now
  stages the release exactly as the publish step does.
- Nothing verified the lockfiles of `seam-py`, `seam-js` and `seam-wasm`. All
  three are excluded from the workspace, so the root `cargo metadata --locked`
  never opened any of them, and `0.1.2` shipped with all three still naming
  `0.1.1`. Harmless in itself — cargo and maturin rewrite a lockfile as they
  build — but a lockfile that only becomes correct during the build is not
  locked. CI now checks each one.

## 0.1.2 — 2026-09-03

### Fixed

- **The Linux wheel excluded far more than it meant to.** `maturin build` ran
  directly on the CI runner, so the wheel was tagged for whatever glibc that
  image happened to have: `manylinux_2_34`. That is Ubuntu 22.04 and RHEL 9 and
  nothing older, while the README promised "Linux x64". It is now linked
  through zig against glibc 2.17 — `manylinux2014` — so Ubuntu 14.04 onwards,
  Debian 8 onwards, RHEL 7 onwards and Amazon Linux 2 install again. The
  release now fails if the wheel is ever tagged higher than that.

  Alpine and musl remain unsupported; `seam-schema-wasm` is the answer there.

## 0.1.1 — 2026-09-01

### Fixed

- **`seam-schema@0.1.0` on npm was broken, and this is the fix.** The package
  reached the registry without `native.js`, the loader that `index.js` requires
  on its first line, so it threw on `require` for everyone who installed it.
  Only the npm package was affected: the wheel and the crate were fine.

## 0.1.0 — 2026-09-01

First release. `seam-core` on crates.io, `seam-schema` on PyPI and npm, and
`seam-schema-wasm` for the browser.

Phases 1 through 3 of [Scope](README.md#scope): the `.seam` parser, the JSON
parser, validation, the four-state absent/null matrix, tagged unions, named
string formats, `abi3` wheels covering Python 3.9 and up, a native Node module
with `bigint` for 64-bit integers, and the same engine compiled to
WebAssembly. The conformance suite runs from Rust, Python, Node and
WebAssembly against the same case files.

**Do not install `seam-schema@0.1.0` from npm.** See 0.1.1.

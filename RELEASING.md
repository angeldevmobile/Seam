# Releasing

Four packages, built from one repository, published together:

| registry | package | artefact |
|---|---|---|
| crates.io | `seam-core` | source |
| PyPI | `seam-schema` | one `abi3` wheel per platform |
| npm | `seam-schema` | a native module per platform |
| npm | `seam-schema-wasm` | one `.wasm`, the same everywhere |

They go out in one version, together, because the conformance suite only means
anything if every binding is the same engine. A binding released against an
older `seam-core` would be exactly the drift this project exists to prevent.

## Before the first release, once

These are account-level things nobody can do from a checkout.

1. **crates.io** — a token with publish rights, stored as the
   `CARGO_REGISTRY_TOKEN` repository secret. Verify `seam-core` is still
   unclaimed; names can be taken between now and then.
2. **PyPI** — configure a *trusted publisher* for this repository and the
   `release.yml` workflow. Trusted publishing uses a short-lived OIDC token, so
   there is nothing to store, leak or rotate. This is why the workflow asks for
   `id-token: write`.
3. **npm** — an automation token as the `NPM_TOKEN` secret. It must be able to
   publish `seam-schema`, `seam-schema-wasm`, and the three per-platform
   packages napi creates (`seam-schema-linux-x64-gnu`,
   `seam-schema-darwin-arm64`, `seam-schema-win32-x64-msvc`).
4. **A `release` environment** in the repository settings, so the publishing job
   can be held for approval. The workflow already targets it.

## Every release

### 1. Decide the version

`0.x` says the contract can still move, which is what
[`spec/mapping.md`](spec/mapping.md) already says of itself: *"Status: draft.
Nothing is frozen until 1.0."* Publishing `1.0.0` would freeze the error codes
and the `.seam` grammar, and neither is settled.

Within `0.x`, treat a renamed or removed error code, or a change to which
condition produces one, as breaking — the spec says so — and bump the minor.

### 2. Set it in one place per manifest

```
Cargo.toml            version = "..."     # the workspace, and seam-core with it
seam-js/Cargo.toml
seam-py/Cargo.toml                        # maturin reads the wheel version here
seam-wasm/Cargo.toml
seam-js/package.json
seam-wasm/package.json
```

Then refresh the four lockfiles, or CI's `lockfile` job will fail:

```bash
cargo update -p seam-core --precise <version>
for c in seam-js seam-py seam-wasm; do cargo metadata --manifest-path $c/Cargo.toml >/dev/null; done
```

The release workflow refuses to build if any of those six disagree, or if the
tag does not match them. That check runs first, before anything is compiled.

### 3. Rehearse

Actions → **Release** → *Run workflow*, leaving `publish` unchecked.

This builds every artefact on every platform, runs the full test suite on each,
and then installs all four packages from their packaged form into scratch
directories and uses them. Nothing reaches a registry. It is the same path a
real release takes, stopping one step short.

Locally, the last part of that is:

```bash
node scripts/packaging-smoke.mjs
```

### 4. Tag

```bash
git tag v<version>
git push origin v<version>
```

The tag triggers the same build and the same smoke test, and then publishes:
crates.io first, because the bindings are useless without the engine and it is
the one that cannot be yanked once others depend on it, then PyPI, then the two
npm packages.

## Why the rehearsal exists

**Publishing cannot be undone.** crates.io does not delete. npm and PyPI allow
a narrow window, and even then the version number is burnt: `0.1.0` can never
mean anything else.

Every test in this repository except one runs inside the checkout, where
`../../conformance` exists and the native module sits where the build left it.
None of that is true of an installed package, and the failures that follow are
the ones that ruin a release: a file missing from `files` or `include`, a
dependency nobody declared, a path that only ever worked in a checkout.
`scripts/packaging-smoke.mjs` is the one test that does not run inside the
checkout, and it is the gate.

## Platforms

The native packages cover three targets:

```
x86_64-unknown-linux-gnu
aarch64-apple-darwin
x86_64-pc-windows-msvc
```

Which leaves out Intel Macs, ARM Linux and ARM Windows. On those, `pip install`
and `npm install` fail rather than falling back, because there is no pure
implementation to fall back to. `seam-schema-wasm` runs anywhere and is the
answer for a platform not on that list.

Adding targets is a matrix entry plus cross-compilation, and is worth doing
before this is something people depend on rather than after.

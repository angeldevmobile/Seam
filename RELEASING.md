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

## The accounts, already set up

Done for `0.1.0`, recorded here because each one has a way of expiring.

1. **crates.io**: a token with `publish-new` and `publish-update`, stored as
   the `CARGO_REGISTRY_TOKEN` secret. **It expires**; when it does, the release
   fails on authentication and the fix is a new token, not a code change.
2. **PyPI**: a *trusted publisher* for `angeldevmobile/Seam`, workflow
   `release.yml`, environment `release`. No token: PyPI accepts a short-lived
   OIDC token minted per run, which is why the workflow asks for
   `id-token: write`. Nothing here expires.
3. **npm**: a granular token with *bypass 2FA*, as the `NPM_TOKEN` secret,
   covering all packages because the five did not exist when it was made.
   **It expires, and it is also on a deadline**: npm is restricting tokens that
   bypass 2FA, with direct publishing cut off in January 2027. See *Migrating
   npm to trusted publishing* below.
4. **A `release` environment** with required reviewers, so publishing waits for
   an approval instead of firing on its own.

## Migrating npm to trusted publishing

Not optional past January 2027, and possible now that the packages exist.
npm has no equivalent of PyPI's "pending publisher", so the first release had
to use a token.

For each of the five packages (`seam-schema`, `seam-schema-wasm`, and the three
per-platform ones), configure a trusted publisher on npmjs.com pointing at
`angeldevmobile/Seam`, workflow `release.yml`, environment `release`. Then drop
`NODE_AUTH_TOKEN` from the workflow and revoke the token. Requires npm CLI
11.5.1 or later on the runner.

Doing this removes the last credential that can leak, and the last one that
expires.

## Every release

### 1. Decide the version

`0.x` says the contract can still move, which is what
[`spec/mapping.md`](spec/mapping.md) already says of itself: *"Status: draft.
Nothing is frozen until 1.0."* Publishing `1.0.0` would freeze the error codes
and the `.seam` grammar, and neither is settled.

Within `0.x`, treat a renamed or removed error code, or a change to which
condition produces one, as breaking, because the spec says so, and bump the minor.

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

Tag last, and tag `main` as it stands, including the README changes. A tag is
the only version of the source anyone can fetch by name, so a doc fix landed
after it is a fix nobody who checks out `v<version>` will read. `v0.1.1` was
tagged one commit before the README stopped saying the project was unpublished,
which is exactly the sentence someone arriving at that tag most needed to be
right.

```bash
git tag v<version>
git push origin v<version>
```

The tag triggers the same build and the same smoke test, and then publishes:
crates.io first, because the bindings are useless without the engine and it is
the one that cannot be yanked once others depend on it, then PyPI, then the two
npm packages.

**Each publish is skipped when that version is already on its registry.** A
release that fails halfway leaves some registries done and others not, and the
only sane fix is re-running it, which works only if republishing an existing
version is a no-op rather than an error. `0.1.0` failed twice this way before
that was true: once on an unverified crates.io email, once on a flag.

Re-running is also the one thing to get right when a release fails: use
**Run workflow** from `main`, not *Re-run failed jobs*. A re-run replays the
original commit, so it will not contain the fix you just pushed.

## What 0.1.0 taught, at the cost of a broken version

`seam-schema@0.1.0` reached npm without `native.js`, the loader that `index.js`
requires on its first line. It threw on `require` for anyone who installed it.
Every test was green.

The direct cause was small: `napi build` writes `native.js`, and the publish
job never runs it, because it only downloads `.node` binaries. The real cause was the
gate. `scripts/packaging-smoke.mjs` ran after `npm run build`, so it packed a
directory that had the file and tested a package **the release never produces**.

> A gate that rebuilds the artefact is not testing the artefact.

Two things changed, and both are worth keeping:

- The smoke job assembles `seam-js` from the same artefacts the publish job
  uses, and does not build.
- The contents of each tarball are asserted before installing it, so a missing
  file fails loudly rather than at somebody else's `require`.

That assertion was verified in both directions: with `native.js` removed the
smoke test fails and says *"do not publish"*. A check that has never been seen
to fail is not known to work.

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

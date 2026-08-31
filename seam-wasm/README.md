# seam-schema-wasm

The browser build of [Seam](https://github.com/angeldevmobile/Seam): one schema,
every language, no drift.

```js
import { Schema } from 'seam-schema-wasm'

const schema = await Schema.parse(await (await fetch('/contracts/user.seam')).text())
const user = schema.validator('User')

const res = await fetch('/api/user/1')
const out = user.validate(await res.arrayBuffer())   // not res.json()
```

Same engine as the Rust, Python and Node bindings — the same compiled
`seam-core`, held to the same conformance suite, case for case.

## Why `arrayBuffer()` and not `json()`

```js
JSON.parse('{"id": 9007199254740993}').id   // 9007199254740992, silently
```

By the time `JSON.parse` has run, the number is already wrong and nothing
afterwards brings the bits back. So **this package takes bytes, not objects**.
Hand it the response body and Seam parses the JSON itself:

```js
const out = user.validate(await res.arrayBuffer())
typeof out.id      // 'bigint'
out.id             // 9007199254740993n
```

Passing an already-parsed object throws a `TypeError` that says so. That is not
an oversight to be fixed later: accepting one would offer, in the runtime where
the problem is worst, the exact path Seam exists to avoid.

`validate` takes a `Uint8Array`, an `ArrayBuffer`, any typed-array view, or a
string.

## Two differences from the Node package

Everything else is identical — same class names, same `issues`, same
`SeamValidationError`, same `path` and `code`. These two are forced by the
medium rather than chosen:

1. **`Schema.parse` is async.** WebAssembly this size cannot be compiled
   synchronously on a browser's main thread, so the module is instantiated on
   first use. It is one `await`, where you were already awaiting the fetch of
   the `.seam` file. Call `ready()` at startup if you would rather pay it
   before the first request than during it.
2. **There is no `Schema.load`.** A browser has no filesystem, and taking a URL
   would fold fetching into validation. Fetch the file, or let your bundler
   inline it, and pass the text.

## Bounding what one call can cost

WebAssembly memory grows and is never returned to the host, so a single
oversized payload raises a tab's floor for the rest of its life. The engine's
limits bound the *shape* of a document — nesting, item counts, key counts,
string length — and this package adds one that bounds the *document*:

```js
schema.validator('User', { maxBytes: 256 * 1024, maxItems: 100 })
```

`maxBytes` defaults to 8 MiB and is checked before anything is copied into the
module, so an oversized payload costs nothing. It is specific to this binding,
for a cost that is specific to this binding.

## Types

`seam typegen` generates one file that serves this package and the Node one:

```ts
import { Schema } from 'seam-schema-wasm'
import type { UserTypes, User } from './contracts/user.types'

const schema = await Schema.parse<UserTypes>(source)
const user: User = schema.validator('User').validate(bytes)
```

A misspelled type name is a compile error. Without the generated map everything
still works and `validate` returns `unknown`, which is the honest type for a
payload nothing has described.

## Size

What a browser downloads, measured by `npm run size`:

| file | raw | gzip | brotli |
|---|---:|---:|---:|
| `seam_wasm_bg.wasm` | 129.1 KiB | 64.7 KiB | 56.2 KiB |
| glue | 16.7 KiB | 4.0 KiB | 3.5 KiB |
| wrapper | 7.1 KiB | 2.8 KiB | 2.3 KiB |
| **total** | **152.9 KiB** | **71.6 KiB** | **62.0 KiB** |

The engine is a `.seam` parser, a JSON parser, a validator and five string
formats, and it carries no dependencies. Rerun the script rather than trusting
the table.

The build runs `wasm-opt -Oz`. On the module before formats it cut 128.2 KiB to
113.9, an 11% saving that was worth about 1.8% compressed, because brotli was
already finding most of that redundancy — worth doing, not the headline it
looks like.

**The five formats cost about 7 KiB brotli**, which is the number to weigh
against a general `@pattern`. A linear regex engine is two orders of magnitude
more than that in a browser bundle, and a backtracking one would break the
bound on hostile input. Seven kilobytes for the cases people actually write is
the trade this project takes.

## Status

Early development, not yet on npm. Build from a checkout:

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.127
cd seam-wasm && npm install && npm run build && npm test
```

The conformance suite runs against the `.wasm` under Node — the same binary a
browser gets, without putting a browser in the critical path of CI. A separate
smoke test drives a real Chromium over HTTP, because loading is the one thing
Node cannot check; it skips when no browser is installed:

```bash
npx playwright install chromium
node --test __test__/browser.test.mjs
```

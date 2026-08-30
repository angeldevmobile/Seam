# seam

Node bindings for [Seam](https://github.com/angeldevmobile/Seam): one schema,
every language, no drift.

```js
const { Schema } = require('seam')

const user = Schema.load('contracts/user.seam').validator('User')
const out = user.validate(rawRequestBody)   // a Buffer, a string, or an object
```

## Hand it the bytes

```js
JSON.parse('{"id": 9007199254740993}').id   // 9007199254740992, silently
```

`JSON.parse` cannot hold an integer past 2^53, and by the time a validator sees
the value the bits are gone. So Seam reads the JSON itself, and a 64-bit field
comes back as a `bigint`:

```js
const out = user.validate(Buffer.from('{"id": 9007199254740993, ...}'))
typeof out.id      // 'bigint'
out.id             // 9007199254740993n
```

If you pass a plain object instead, a `number` past 2^53 in a 64-bit field is
**refused** rather than validated as the wrong value:

```js
user.validate({ id: 9007199254740993, ... })   // throws: id, unsafe_integer
```

There is nothing else Seam could honestly do: the caller's own value is already
not the one that was sent.

## The three empty states

JavaScript has `undefined`, `null` and a missing property, and Seam keeps them
apart. `undefined` reads as absent, which is what `JSON.stringify` does with it:

```js
'bio' in out          // was the key sent at all?
out.avatar === null   // was it sent as null?
```

## Dates

A `Date` field is a calendar date and comes back as its ISO **string**. A
`DateTime` comes back as a JavaScript `Date`. Pushing a calendar date through
an instant is the origin of every off-by-one-day bug at a timezone boundary, so
Seam does not do it.

A `DateTime` without an offset is rejected. Seam never assumes local time.

## Errors

```js
const { SeamValidationError } = require('seam')

try {
  user.validate(payload)
} catch (e) {
  if (e instanceof SeamValidationError) {
    e.issues        // every failure, not just the first
    e.path, e.code  // the first one, for the common case
  }
}
```

`path` and `code` are stable API. `message` is the summary, as JavaScript
expects of an `Error`; an individual issue's own text is `issues[0].message`.
Nothing is built until it is read.

## Limits

Untrusted input is bounded whether or not you ask. Tighten them to what a
legitimate request looks like:

```js
schema.validator('User', { maxItems: 100, maxStringBytes: 4096 })
```

## Generated TypeScript

```bash
npx seam typegen contracts/user.seam     # writes contracts/user.types.ts
```

The generated file is **types only**: no imports, no code, nothing to run.
Delete it and everything still works; you lose static checking and nothing
else. Validation happens in the engine, against the `.seam` file, at runtime.

```ts
import { Schema } from 'seam'
import type { UserTypes } from './contracts/user.types'

const schema = Schema.load<UserTypes>('contracts/user.seam')
const user = schema.validator('User').validate(rawRequestBody)

user.id            // bigint
user.plan          // 'free' | 'pro' | 'enterprise'
user.bio           // string | undefined
```

Passing the generated map to `Schema.load` types every validator it hands out,
so `validator('Usr')` is a compile error rather than a runtime one. Without it
nothing breaks and `validate` returns `unknown`, which is the honest type for a
payload nothing has described.

The mapping is the one TypeScript already had words for:

| `.seam` | TypeScript |
|---|---|
| `String` | `name: string` |
| `String?` | `nickname: string \| null` |
| `optional String` | `bio?: string` |
| `optional String?` | `avatar?: string \| null` |
| `u8`-`u32`, `i8`-`i32`, `f64` | `number` |
| `u64`, `i64` | `bigint` |
| `Date` | `string` |
| `DateTime` | `Date` |
| `enum { free, pro }` | `'free' \| 'pro'` |
| `[String?]` | `(string \| null)[]` |

`?:` is the absence axis and `| null` is the nullability axis, which is the
same distinction `NotRequired` draws in Python. They are independent, and a
field may carry both.

In CI, `--check` fails if a generated file has fallen behind its schema:

```bash
npx seam typegen --check contracts/*.seam
```

## Status

Early development, not yet on npm. **Node 20 or newer**: the compiled module
targets Node-API 6 and would run on 18, but the build tool needs 20, and a
version nothing tests is not a version this claims to support.

Build from a checkout:

```bash
cd seam-js && npm install && npm run build && npm test
```

The `seam` command is installed by both this package and the Python one. They
take the same subcommand and the same flags on purpose, but if you install both
globally, whichever is first on `PATH` wins; `npx seam` always reaches this
one.

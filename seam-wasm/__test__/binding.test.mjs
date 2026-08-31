// What is specific to the wasm build, and the shape of what comes back.
//
// The conformance suite already proves this binding agrees with the other
// three on the rules. What is left is everything the medium forces: bytes
// only, an async first use, and a bound on how much memory one call can make
// the module keep.

import { test } from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

import { Schema, SeamValidationError, DEFAULT_MAX_BYTES, ready } from '../index.js'

const HERE = dirname(fileURLToPath(import.meta.url))
const CONFORMANCE = join(HERE, '..', '..', 'conformance')

const source = readFileSync(join(CONFORMANCE, 'schemas', 'user.seam'), 'utf8')
const user = await Schema.parse(source)
const validate = user.validator('User')

const RAW = '{"id": 9007199254740993, "name": "Gabriel", "plan": "pro", "nickname": null}'

/** `assert.throws` does not hand back the error, and these tests read it. */
function caught(fn, expected) {
  try {
    fn()
  } catch (e) {
    if (expected && !(e instanceof expected)) {
      throw new Error(`expected ${expected.name}, got ${e?.constructor?.name}: ${e?.message}`)
    }
    return e
  }
  throw new Error('expected this to throw, and it did not')
}

function codes(payload) {
  try {
    validate.validate(payload)
    return []
  } catch (e) {
    if (!(e instanceof SeamValidationError)) throw e
    return e.issues.map((i) => `${i.path}:${i.code}`)
  }
}

// --- the promise this project is built on -----------------------------------

test('a 64-bit integer survives raw JSON and comes back as a bigint', () => {
  const out = validate.validate(Buffer.from(RAW))
  assert.equal(typeof out.id, 'bigint')
  assert.equal(out.id, 9007199254740993n)
})

test('JSON.parse would already have lost it', () => {
  // Not a test of Seam. It is the reason this package refuses objects, and it
  // fails loudly here if JavaScript ever stops being like this.
  assert.equal(JSON.parse('{"id": 9007199254740993}').id, 9007199254740992)
})

test('an integer past 64 bits is too wide, not merely out of range', () => {
  const bytes = Buffer.from('{"id": 18446744073709551616, "name": "Gabriel", "plan": "pro", "nickname": null}')
  assert.deepEqual(codes(bytes), ['id:integer_too_wide'])
})

// --- bytes, and nothing but bytes -------------------------------------------

test('every byte-shaped payload is accepted', () => {
  const expected = 9007199254740993n
  const buf = Buffer.from(RAW)
  assert.equal(validate.validate(RAW).id, expected, 'string')
  assert.equal(validate.validate(buf).id, expected, 'Buffer')
  assert.equal(validate.validate(new Uint8Array(buf)).id, expected, 'Uint8Array')
  assert.equal(
    validate.validate(buf.buffer.slice(buf.byteOffset, buf.byteOffset + buf.byteLength)).id,
    expected,
    'ArrayBuffer',
  )
})

test('a typed-array view reads only its own window', () => {
  // A Buffer from a pool shares its ArrayBuffer with unrelated bytes, so a
  // binding that ignored byteOffset would validate somebody else's data.
  const padded = Buffer.concat([Buffer.from('xxxxx'), Buffer.from(RAW), Buffer.from('yyyyy')])
  const view = new Uint8Array(padded.buffer, padded.byteOffset + 5, RAW.length)
  assert.equal(validate.validate(view).id, 9007199254740993n)
})

test('an already-parsed object is refused, by name and with a way out', () => {
  // The one thing this package must not accept: its 64-bit integers are
  // already wrong, and validating them would bless a value nobody can trust.
  const e = caught(() => validate.validate({ id: 1, name: 'Gabriel' }), TypeError)
  assert.match(e.message, /bytes or text/)
  assert.match(e.message, /arrayBuffer/)
})

test('null and undefined are refused as payloads, not treated as empty', () => {
  assert.throws(() => validate.validate(null), TypeError)
  assert.throws(() => validate.validate(undefined), TypeError)
})

test('malformed JSON is an error, not a validation failure', () => {
  const e = caught(() => validate.validate('{ not json'))
  assert.ok(!(e instanceof SeamValidationError))
})

// --- bounding what one call can cost ----------------------------------------

test('a payload over maxBytes is refused before it reaches the module', () => {
  const small = user.validator('User', { maxBytes: 16 })
  const e = caught(() => small.validate(Buffer.from(RAW)), RangeError)
  assert.match(e.message, /over the limit of 16/)
})

test('maxBytes defaults to the documented value and can be raised', () => {
  assert.equal(DEFAULT_MAX_BYTES, 8 * 1024 * 1024)
  const big = user.validator('User', { maxBytes: 1024 })
  assert.equal(big.validate(Buffer.from(RAW)).id, 9007199254740993n)
})

test('a schema-wide maxBytes applies to every validator it hands out', async () => {
  const tight = await Schema.parse(source, { maxBytes: 16 })
  assert.throws(() => tight.validator('User').validate(Buffer.from(RAW)), RangeError)
})

test('a nonsense maxBytes is refused rather than ignored', async () => {
  await assert.rejects(() => Schema.parse(source, { maxBytes: 0 }), RangeError)
  await assert.rejects(() => Schema.parse(source, { maxBytes: 1.5 }), RangeError)
})

test('the engine limits still apply, and a bad one is reported', () => {
  assert.deepEqual(codes(Buffer.from(RAW)), [])
  const shallow = user.validator('User', { maxDepth: 1 })
  assert.deepEqual(shallow.validate(Buffer.from(RAW)).id, 9007199254740993n)
  assert.throws(() => user.validator('User', { maxItems: 'lots' }))
})

// --- the async seam ---------------------------------------------------------

test('ready is idempotent and returns the same promise', async () => {
  const a = ready()
  const b = ready()
  assert.equal(a, b)
  await a
})

test('parse rejects a broken schema rather than returning something unusable', async () => {
  await assert.rejects(() => Schema.parse('schema A { x: Nope }'))
})

test('a type the schema does not declare is refused when binding', () => {
  assert.throws(() => user.validator('Nope'))
})

// --- the shape of what comes back -------------------------------------------

test('absent stays absent, and null stays null', () => {
  const out = validate.validate(Buffer.from(RAW))
  assert.equal('bio' in out, false)
  assert.equal('nickname' in out, true)
  assert.equal(out.nickname, null)
})

test('a DateTime is a Date and a Date is its ISO string', () => {
  // JS has no date-only type. Pushing a calendar date through an instant is
  // what produces off-by-one-day bugs, so it stays the string it arrived as.
  const out = validate.validate(
    Buffer.from(
      '{"id":1,"name":"Gabriel","plan":"pro","nickname":null,' +
        '"last_seen":"2026-08-30T12:00:00Z","signup_date":"2026-08-30"}',
    ),
  )
  assert.ok(out.last_seen instanceof Date)
  assert.equal(out.last_seen.toISOString(), '2026-08-30T12:00:00.000Z')
  assert.equal(typeof out.signup_date, 'string')
  assert.equal(out.signup_date, '2026-08-30')
})

test('a tagged union keeps its tag and picks its variant', async () => {
  const events = await Schema.parse(
    readFileSync(join(CONFORMANCE, 'schemas', 'events.seam'), 'utf8'),
  )
  const out = events
    .validator('Event')
    .validate('{"type":"created","who":"Gabriel","amount":9007199254740993}')
  assert.equal(out.type, 'created')
  assert.equal(out.amount, 9007199254740993n)
})

// --- errors -----------------------------------------------------------------

test('a validation failure is a real Error with every issue', () => {
  const e = caught(
    () => validate.validate('{"id":-1,"name":"ab","plan":"nope","nickname":null}'),
    SeamValidationError,
  )
  assert.ok(e instanceof Error)
  assert.ok(e.stack)
  assert.deepEqual(
    e.issues.map((i) => i.code),
    ['out_of_range', 'too_short', 'not_in_enum'],
  )
  assert.equal(e.path, 'id')
  assert.equal(e.code, 'out_of_range')
  assert.match(e.message, /and 2 more/)
})

test('a limit is a verdict with a code, not a parse error', () => {
  // This binding only has the bytes path, so the code is the whole of what a
  // caller can act on. Reporting it as a parse error would have hidden it.
  const tight = user.validator('User', { maxStringBytes: 4 })
  const e = caught(() => tight.validate(Buffer.from(RAW)), SeamValidationError)
  assert.deepEqual(
    [...new Set(e.issues.map((i) => i.code))],
    ['size_exceeded'],
  )
})

// The traps that are specific to JavaScript, and the shape of what comes back.

import { test } from 'node:test'
import assert from 'node:assert/strict'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { createRequire } from 'node:module'

const require = createRequire(import.meta.url)
const { Schema, SeamValidationError } = require('../index.js')

const HERE = dirname(fileURLToPath(import.meta.url))
const user = Schema.load(join(HERE, '..', '..', 'conformance', 'schemas', 'user.seam'))
const validate = user.validator('User')

const base = () => ({ id: 1n, name: 'Gabriel', plan: 'pro', nickname: null })
const raw = (o) => Buffer.from(JSON.stringify(o))

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
  const bytes = Buffer.from(
    '{"id": 9007199254740993, "name": "Gabriel", "plan": "pro", "nickname": null}',
  )
  const out = validate.validate(bytes)
  assert.equal(typeof out.id, 'bigint')
  assert.equal(out.id, 9007199254740993n)
})

test('JSON.parse would already have lost it', () => {
  // Not a test of Seam. It is the reason Seam reads the bytes itself, and it
  // fails loudly here if JavaScript ever stops being like this.
  const parsed = JSON.parse('{"id": 9007199254740993}')
  assert.equal(parsed.id, 9007199254740992)
  assert.notEqual(BigInt(parsed.id), 9007199254740993n)
})

test('a number past 2^53 is refused rather than validated as the wrong value', () => {
  // The caller's own value is already wrong; validating it would bless a lie.
  assert.deepEqual(codes({ ...base(), id: 9007199254740993 }), ['id:unsafe_integer'])
})

test('a number inside the safe range is fine', () => {
  assert.deepEqual(codes({ ...base(), id: 42 }), [])
})

test('an integer past 64 bits is too wide, not merely unsafe', () => {
  const bytes = Buffer.from(
    '{"id": 18446744073709551616, "name": "Gabriel", "plan": "pro", "nickname": null}',
  )
  assert.deepEqual(codes(bytes), ['id:integer_too_wide'])
})

// --- the three states JavaScript has ----------------------------------------

test('undefined means absent, the way JSON.stringify reads it', () => {
  // `JSON.stringify({bio: undefined})` is `{}`, so absent is the honest reading.
  assert.deepEqual(codes({ ...base(), bio: undefined }), [])
  assert.deepEqual(codes({ ...base(), nickname: undefined }), ['nickname:required'])
})

test('null is not absent', () => {
  assert.deepEqual(codes({ ...base(), bio: null }), ['bio:null_not_allowed'])
  assert.deepEqual(codes({ ...base(), avatar: null }), [])
})

test('an absent field is not a property of the result', () => {
  const out = validate.validate(base())
  assert.equal('bio' in out, false)
  assert.equal('nickname' in out, true)
  assert.equal(out.nickname, null)
})

// --- dates ------------------------------------------------------------------

test('a calendar date stays a string, and never becomes a Date', () => {
  const out = validate.validate(raw({ ...base(), id: 1, signup_date: '2026-08-29' }))
  assert.equal(out.signup_date, '2026-08-29')
  assert.equal(out.signup_date instanceof Date, false)
})

test('a datetime comes back as a Date', () => {
  const out = validate.validate(raw({ ...base(), id: 1, last_seen: '2026-08-29T14:30:00Z' }))
  assert.ok(out.last_seen instanceof Date)
  assert.equal(out.last_seen.toISOString(), '2026-08-29T14:30:00.000Z')
})

test('a datetime without an offset is refused', () => {
  assert.deepEqual(codes({ ...base(), last_seen: '2026-08-29T14:30:00' }), [
    'last_seen:missing_timezone',
  ])
})

test('a JS Date in a calendar-date field is refused', () => {
  // A Date is an instant. Its ISO form carries a time, so it is not a date.
  assert.deepEqual(codes({ ...base(), signup_date: new Date('2026-08-29') }), [
    'signup_date:invalid_date',
  ])
})

// --- errors -----------------------------------------------------------------

test('every issue is reported, not just the first', () => {
  const got = codes({ id: 1n, name: 'ab', plan: 'platinum', nickname: null })
  assert.deepEqual(got.sort(), ['name:too_short', 'plan:not_in_enum'])
})

test('the error is a real Error with a stack', () => {
  try {
    validate.validate({ ...base(), name: 'ab' })
    assert.fail('should have thrown')
  } catch (e) {
    assert.ok(e instanceof Error)
    assert.ok(e.stack.includes('binding.test'))
    assert.equal(e.name, 'SeamValidationError')
    assert.equal(e.path, 'name')
    assert.equal(e.code, 'too_short')
    assert.match(e.message, /name.*too_short/)
  }
})

test('errors carry the path into arrays', () => {
  assert.deepEqual(codes({ ...base(), tags: ['ok', 7] }), ['tags[1]:type_mismatch'])
})

// --- the surface ------------------------------------------------------------

test('a string of JSON works as well as a Buffer', () => {
  const text = '{"id": 1, "name": "Gabriel", "plan": "pro", "nickname": null}'
  assert.deepEqual(validate.validate(text), validate.validate(Buffer.from(text)))
})

test('malformed JSON says where', () => {
  assert.throws(() => validate.validate(Buffer.from('{"id": }')), /1:8/)
})

test('type names, and binding an unknown one fails at bind time', () => {
  assert.deepEqual(user.typeNames(), ['User'])
  assert.throws(() => user.validator('Nope'), /no type named/)
})

test('limits are reachable', () => {
  const tight = user.validator('User', { maxItems: 1 })
  assert.throws(
    () => tight.validate({ ...base(), tags: ['a', 'b'] }),
    (e) => e.code === 'size_exceeded',
  )
})

// --- a limit is a verdict, not a syntax error --------------------------------

test('a limit reports the same code whichever path the payload took', () => {
  // Parsing stops before the value exists, so the path differs; the code does
  // not, and the code is what a service acts on.
  const tight = user.validator('User', { maxStringBytes: 4 })
  const payload = { id: 1n, name: 'Gabriel', plan: 'pro', nickname: null }

  const codesOf = (p) => {
    try {
      tight.validate(p)
      return []
    } catch (e) {
      if (!(e instanceof SeamValidationError)) throw e
      return [...new Set(e.issues.map((i) => i.code))]
    }
  }

  assert.deepEqual(codesOf(raw({ ...payload, id: 1 })), ['size_exceeded'])
  assert.deepEqual(codesOf(payload), ['size_exceeded'])
})

test('malformed JSON is still a parse error, not a verdict', () => {
  // A limit is a statement about the data. Broken syntax is not.
  let thrown
  try {
    validate.validate(Buffer.from('{ nope'))
  } catch (e) {
    thrown = e
  }
  assert.ok(thrown && !(thrown instanceof SeamValidationError))
})

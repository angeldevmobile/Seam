// The shared conformance suite, run from Node.
//
// Same files the Rust and Python runners read. Agreement between the three is
// what "no drift" means.
//
// Cases carrying `input_raw` are fed to the binding as bytes, because
// `JSON.parse` would corrupt an integer past 2^53 while merely loading the
// test file. That a JavaScript runner needs this at all is the point the suite
// is making.

import { test } from 'node:test'
import assert from 'node:assert/strict'
import { readdirSync, readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { createRequire } from 'node:module'

const require = createRequire(import.meta.url)
const { Schema, SeamValidationError } = require('../index.js')

const HERE = dirname(fileURLToPath(import.meta.url))
const CONFORMANCE = join(HERE, '..', '..', 'conformance')

const schemas = new Map()
function load(name) {
  if (!schemas.has(name)) {
    schemas.set(name, Schema.load(join(CONFORMANCE, 'schemas', `${name}.seam`)))
  }
  return schemas.get(name)
}

function collect() {
  const out = []
  for (const file of readdirSync(join(CONFORMANCE, 'cases')).sort()) {
    if (!file.endsWith('.json')) continue
    const doc = JSON.parse(readFileSync(join(CONFORMANCE, 'cases', file), 'utf8'))
    const base = doc.base ?? {}
    for (const c of doc.cases) {
      // Prefer the exact text where a case says the bytes matter.
      const payload = c.input_raw
        ? Buffer.from(c.input_raw)
        : Buffer.from(JSON.stringify({ ...base, ...(c.input ?? {}) }))
      out.push({ name: `${file} :: ${c.name}`, schema: doc.schema, type: doc.type, payload, expect: c.expect })
    }
  }
  return out
}

const CASES = collect()

function found(schemaName, typeName, payload) {
  try {
    load(schemaName).validate(typeName, payload)
    return []
  } catch (e) {
    if (!(e instanceof SeamValidationError)) throw e
    return e.issues.map((i) => `${i.path}:${i.code}`)
  }
}

function expected(expect) {
  if (expect === 'valid') return []
  return (expect.issues ?? []).map((i) => `${i.path}:${i.code}`)
}

test('the suite is not empty', () => {
  assert.ok(CASES.length >= 68, `collected only ${CASES.length}`)
})

for (const c of CASES) {
  test(c.name, () => {
    assert.deepEqual(found(c.schema, c.type, c.payload), expected(c.expect))
  })
}

test('the harness detects a wrong expectation', () => {
  // A harness that cannot fail proves nothing.
  const valid = Buffer.from('{"id": 1, "name": "Gabriel", "plan": "pro", "nickname": null}')
  assert.deepEqual(found('user', 'User', valid), [])
  assert.notDeepEqual(found('user', 'User', valid), ['id:type_mismatch'])
})

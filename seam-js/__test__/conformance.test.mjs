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

// The case files spell limits the way the spec does; this binding takes them
// camelCased, so the one place they are translated is here.
const LIMIT_KEYS = {
  max_depth: 'maxDepth',
  max_items: 'maxItems',
  max_string_bytes: 'maxStringBytes',
  max_object_keys: 'maxObjectKeys',
}

function limitsOf(c) {
  const out = {}
  for (const [from, to] of Object.entries(LIMIT_KEYS)) {
    if (c.limits && c.limits[from] !== undefined) out[to] = c.limits[from]
  }
  return out
}

function collect() {
  const out = []
  for (const file of readdirSync(join(CONFORMANCE, 'cases')).sort()) {
    if (!file.endsWith('.json')) continue
    const doc = JSON.parse(readFileSync(join(CONFORMANCE, 'cases', file), 'utf8'))
    const base = doc.base ?? {}
    for (const c of doc.cases) {
      const merged = { ...base, ...(c.input ?? {}) }
      // `host_value` means feed the binding this runtime's own values, which is
      // the only way to reach the rules about what a host cannot hold. Note
      // that `merged` has already been through JSON.parse by now — for the
      // 2^53 case that corruption is the condition under test, not a flaw.
      const payload = c.host_value
        ? merged
        : c.input_raw
          ? Buffer.from(c.input_raw)
          : Buffer.from(JSON.stringify(merged))
      out.push({
        name: `${file} :: ${c.name}`,
        schema: doc.schema,
        type: doc.type,
        payload,
        // JavaScript numbers are not exact past 2^53, so where a case
        // distinguishes, this runner takes the inexact expectation.
        expect: c.expect_inexact_integers ?? c.expect,
        limits: limitsOf(c),
      })
    }
  }
  return out
}

const CASES = collect()

function found(schemaName, typeName, payload, limits) {
  try {
    load(schemaName).validate(typeName, payload, limits)
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

/** Which codes appeared, not where or how many times. */
function codesOf(found) {
  return [...new Set(found.map((s) => s.slice(s.lastIndexOf(':') + 1)))].sort()
}

test('the suite is not empty', () => {
  assert.ok(CASES.length >= 78, `collected only ${CASES.length}`)
})

for (const c of CASES) {
  test(c.name, () => {
    const got = found(c.schema, c.type, c.payload, c.limits)
    // A limit is caught while parsing, before the value it belongs to exists:
    // a binding fed bytes stops at the first breach and has no path, while one
    // fed host objects finishes the walk and reports each. Both must agree on
    // the code, and the code is the stable API.
    if (c.expect?.codes) {
      assert.deepEqual(codesOf(got), [...c.expect.codes].sort())
    } else {
      assert.deepEqual(got, expected(c.expect))
    }
  })
}

test('the harness detects a wrong expectation', () => {
  // A harness that cannot fail proves nothing.
  const valid = Buffer.from('{"id": 1, "name": "Gabriel", "plan": "pro", "nickname": null}')
  assert.deepEqual(found('user', 'User', valid, {}), [])
  assert.notDeepEqual(found('user', 'User', valid, {}), ['id:type_mismatch'])
})

// The shared conformance suite, run against the wasm build.
//
// Same files the Rust, Python and Node runners read. Agreement between the
// four is what "no drift" means, and a fourth binding is only worth having if
// it is held to the same cases as the other three.
//
// Run under Node rather than in a headless browser: it is the same `.wasm`
// binary either way, and putting a browser in the critical path of CI buys
// flakiness rather than coverage. A separate smoke test covers actually
// loading in a browser.
//
// Every payload reaches the binding as bytes, which is the only path this
// package has. That is not a limitation of the runner: `JSON.parse` would
// corrupt an integer past 2^53 while merely loading the case file.

import { test } from 'node:test'
import assert from 'node:assert/strict'
import { readdirSync, readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

import { Schema, SeamValidationError } from '../index.js'

const HERE = dirname(fileURLToPath(import.meta.url))
const CONFORMANCE = join(HERE, '..', '..', 'conformance')

const schemas = new Map()
async function load(name) {
  if (!schemas.has(name)) {
    const source = readFileSync(join(CONFORMANCE, 'schemas', `${name}.seam`), 'utf8')
    schemas.set(name, await Schema.parse(source))
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
      // Prefer the exact text where a case says the bytes matter.
      const payload = c.input_raw
        ? Buffer.from(c.input_raw)
        : Buffer.from(JSON.stringify({ ...base, ...(c.input ?? {}) }))
      out.push({
        name: `${file} :: ${c.name}`,
        schema: doc.schema,
        type: doc.type,
        payload,
        expect: c.expect,
        limits: limitsOf(c),
        // This package has no object path at all, so a case about what a host
        // does with its own values cannot be reproduced here. Skipped out
        // loud: a case that quietly passed by not running would be worse than
        // one that fails.
        hostValue: c.host_value === true,
      })
    }
  }
  return out
}

const CASES = collect()

function expected(expect) {
  if (expect === 'valid') return []
  return (expect.issues ?? []).map((i) => [i.path, i.code])
}

/**
 * Binding is inside the try on purpose: a binding may report an undeclared
 * type name when the validator is bound rather than when a payload arrives,
 * which is earlier and better, and it is still the same verdict.
 */
function found(schema, typeName, payload, limits) {
  try {
    schema.validator(typeName, limits).validate(payload)
    return []
  } catch (e) {
    if (!(e instanceof SeamValidationError)) throw e
    return e.issues.map((i) => [i.path, i.code])
  }
}

for (const c of CASES) {
  test(c.name, async (t) => {
    if (c.hostValue) {
      return t.skip('this binding takes bytes only, by design')
    }
    const schema = await load(c.schema)
    const got = found(schema, c.type, c.payload, c.limits)
    // A limit is caught while parsing, before the value it belongs to exists:
    // a binding fed bytes stops at the first breach and has no path, while one
    // fed host objects finishes the walk and reports each. Both must agree on
    // the code, and the code is the stable API.
    if (c.expect?.codes) {
      assert.deepEqual(
        [...new Set(got.map(([, code]) => code))].sort(),
        [...c.expect.codes].sort(),
      )
    } else {
      assert.deepEqual(got, expected(c.expect))
    }
  })
}

test('the suite is not empty', () => {
  assert.ok(CASES.length >= 78, `collected only ${CASES.length}`)
})

test('the harness detects a wrong expectation', async () => {
  // A harness that cannot fail proves nothing.
  const schema = await load('user')
  const valid = Buffer.from('{"id":1,"name":"Gabriel","plan":"pro","nickname":null}')
  assert.deepEqual(found(schema, 'User', valid, {}), [])
  assert.notDeepEqual(found(schema, 'User', Buffer.from('{"id":1}'), {}), [])
})

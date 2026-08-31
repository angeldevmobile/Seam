// The generated types are only worth anything if a compiler acts on them, so
// these tests run `tsc` rather than only inspecting strings.
//
// The schema is the same one `seam-py/tests/test_typegen.py` uses, on purpose:
// the two generators read the same `describe()` shape and should disagree only
// where the two languages do.

import { test } from 'node:test'
import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { mkdtempSync, readFileSync, writeFileSync, rmSync, existsSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { createRequire } from 'node:module'

const require = createRequire(import.meta.url)
const { Schema } = require('../index.js')
const { generate, outputPath } = require('../typegen.js')
const { main } = require('../cli.js')

const HERE = dirname(fileURLToPath(import.meta.url))
const PACKAGE = join(HERE, '..')
const TSC = join(PACKAGE, 'node_modules', 'typescript', 'bin', 'tsc')

const SCHEMA = `
schema Address {
  city:    String
  zip:     optional String
}

schema Person {
  name:     String
  age:      u32               @range(0..=130)
  ticket:   u64
  plan:     enum { free, pro }
  home:     Address
  nickname: String?
  bio:      optional String
  avatar:   optional String?
  born:     optional Date
  seen:     optional DateTime
  tags:     [String?]
}
`

/** A throwaway directory holding person.seam and its generated types. */
function project(source = SCHEMA) {
  const dir = mkdtempSync(join(tmpdir(), 'seam-typegen-'))
  const seam = join(dir, 'person.seam')
  writeFileSync(seam, source, 'utf8')
  return { dir, seam }
}

function render(source = SCHEMA) {
  const p = project(source)
  const out = join(p.dir, 'person.types.ts')
  writeFileSync(out, generate(p.seam), 'utf8')
  return { ...p, out, body: readFileSync(out, 'utf8') }
}

// --- the mapping ------------------------------------------------------------

test('the four presence states render distinctly', () => {
  const { body } = render()
  // Nullability is `| null`; absence is `?`. Two axes, two spellings, and a
  // field can carry both.
  assert.match(body, /^ {2}name: string$/m)
  assert.match(body, /^ {2}nickname: string \| null$/m)
  assert.match(body, /^ {2}bio\?: string$/m)
  assert.match(body, /^ {2}avatar\?: string \| null$/m)
})

test('types render as their TypeScript counterparts', () => {
  const { body } = render()
  assert.match(body, /^ {2}age: number$/m)
  assert.match(body, /^ {2}plan: 'free' \| 'pro'$/m)
  assert.match(body, /^ {2}home: Address$/m)
  assert.match(body, /^ {2}tags: \(string \| null\)\[\]$/m)
})

test('a 64-bit integer is a bigint and a narrower one is a number', () => {
  // The whole reason this binding exists, carried into the type system.
  const { body } = render()
  assert.match(body, /^ {2}ticket: bigint$/m)
  assert.match(body, /^ {2}age: number$/m)
})

test('a Date is a string and a DateTime is a Date', () => {
  // JavaScript has no date-only type. Pushing a calendar date through an
  // instant is what produces off-by-one-day bugs, so it stays the ISO string.
  const { body } = render()
  assert.match(body, /^ {2}born\?: string$/m)
  assert.match(body, /^ {2}seen\?: Date$/m)
})

test('a reference may point at a type defined earlier or later', () => {
  // `Address` is sorted before `Person`, but TypeScript checks the whole file
  // before resolving, so neither order needs a forward declaration.
  const { body } = render()
  assert.ok(body.indexOf('interface Address') < body.indexOf('interface Person'))
  assert.match(body, /^ {2}home: Address$/m)
})

test('the type map names every declared type', () => {
  const { body } = render()
  assert.match(body, /export interface PersonTypes \{\n {2}Address: Address\n {2}Person: Person\n\}/)
})

test('the map name steps aside if the schema declares that name itself', () => {
  const { body } = render('schema PersonTypes { x: u8 }\n')
  assert.match(body, /export interface PersonSchemaTypes \{/)
})

// --- the point: does a compiler act on it -----------------------------------

/** Type-checks a directory, with `seam` resolving to this package's types. */
function tsc(dir, files) {
  writeFileSync(
    join(dir, 'tsconfig.json'),
    JSON.stringify({
      compilerOptions: {
        strict: true,
        noEmit: true,
        target: 'es2022',
        module: 'preserve',
        moduleResolution: 'bundler',
        skipLibCheck: true,
        paths: { seam: [join(PACKAGE, 'index.d.ts')] },
      },
      files,
    }),
    'utf8',
  )
  try {
    execFileSync(process.execPath, [TSC, '-p', dir], { encoding: 'utf8', stdio: 'pipe' })
    return { ok: true, output: '' }
  } catch (e) {
    return { ok: false, output: `${e.stdout ?? ''}${e.stderr ?? ''}` }
  }
}

/** Writes a caller alongside the generated types and type-checks both. */
function check(body) {
  const { dir } = render()
  writeFileSync(join(dir, 'use_it.ts'), body, 'utf8')
  return tsc(dir, ['person.types.ts', 'use_it.ts'])
}

const USE = `
import { Schema } from 'seam'
import type { PersonTypes, Person } from './person.types'

const schema = Schema.load<PersonTypes>('person.seam')
const person: Person = schema.validator('Person').validate(new Uint8Array())
`

test('the generated file type-checks on its own', () => {
  const { dir } = render()
  const result = tsc(dir, ['person.types.ts'])
  assert.ok(result.ok, result.output)
})

test('a validator bound through the generated map is typed', () => {
  const result = check(`${USE}\nconst n: string = person.name\n`)
  assert.ok(result.ok, result.output)
})

test('tsc catches a misspelled key', () => {
  const result = check(`${USE}\nconsole.log(person.nmae)\n`)
  assert.ok(!result.ok)
  assert.match(result.output, /nmae/)
})

test('tsc catches a wrong type', () => {
  const result = check(`${USE}\nconst age: string = person.age\n`)
  assert.ok(!result.ok)
})

test('tsc catches reading an absent field as if it were there', () => {
  // `bio?: string` is `string | undefined`, which is the point of the axis.
  const result = check(`${USE}\nconst bio: string = person.bio\n`)
  assert.ok(!result.ok)
})

test('tsc catches a 64-bit field used as a number', () => {
  const result = check(`${USE}\nconst ticket: number = person.ticket\n`)
  assert.ok(!result.ok)
})

test('tsc catches a type name the schema does not declare', () => {
  // What the generated map buys over a cast: the string is checked too.
  const result = check(`
import { Schema } from 'seam'
import type { PersonTypes } from './person.types'

Schema.load<PersonTypes>('person.seam').validator('Persn')
`)
  assert.ok(!result.ok)
  assert.match(result.output, /Persn/)
})

test('a schema loaded without the map still checks, as unknown', () => {
  // The generated file is optional. Without it nothing breaks; `validate`
  // returns `unknown`, which is the honest type for an undescribed payload.
  const result = check(`
import { Schema } from 'seam'

const out: unknown = Schema.load('person.seam').validator('Anything').validate({})
void out
`)
  assert.ok(result.ok, result.output)
})

// --- the CLI ----------------------------------------------------------------

test('typegen writes next to the schema', () => {
  const { dir, seam } = project()
  assert.equal(main(['typegen', seam]), 0)
  assert.ok(existsSync(join(dir, 'person.types.ts')))
  assert.equal(outputPath(seam), join(dir, 'person.types.ts'))
})

test('--output takes a single schema', () => {
  const { seam } = project()
  assert.notEqual(main(['typegen', '-o', 'x.ts', seam, seam]), 0)
})

test('-o writes where it is told, creating the directory', () => {
  const { dir, seam } = project()
  const target = join(dir, 'generated', 'types.ts')
  assert.equal(main(['typegen', '-o', target, seam]), 0)
  assert.ok(existsSync(target))
})

test('--check fails when the generated file is missing or stale', () => {
  const { seam } = project()

  // Missing.
  assert.equal(main(['typegen', '--check', seam]), 1)

  // Present and current.
  assert.equal(main(['typegen', seam]), 0)
  assert.equal(main(['typegen', '--check', seam]), 0)

  // The schema moves on and the generated file does not.
  writeFileSync(seam, `${SCHEMA}\nschema Extra { x: u8 }\n`, 'utf8')
  assert.equal(main(['typegen', '--check', seam]), 1)
})

test('a broken schema reports rather than writing', () => {
  const { dir } = project()
  const bad = join(dir, 'bad.seam')
  writeFileSync(bad, 'schema A { x: Nope }', 'utf8')

  assert.equal(main(['typegen', bad]), 1)
  assert.ok(!existsSync(join(dir, 'bad.types.ts')))
})

test('the generated bytes are the same on every platform', () => {
  // `--check` compares text, so a CRLF anywhere would fail CI on Windows only.
  const { dir, seam } = project()
  main(['typegen', seam])
  const raw = readFileSync(join(dir, 'person.types.ts'))
  assert.ok(!raw.includes('\r'.charCodeAt(0)))
})

// --- the generated file is not a second source of truth ---------------------

test('deleting the generated file costs only static checking', () => {
  const { seam, out } = render()
  const payload = {
    name: 'Gabriel',
    age: 30,
    ticket: 1n,
    plan: 'pro',
    home: { city: 'Lima' },
    nickname: null,
    tags: ['a', null],
  }
  const before = Schema.load(seam).validate('Person', payload)

  rmSync(out)
  const after = Schema.load(seam).validate('Person', payload)
  assert.deepEqual(before, after)
})

// --- tagged unions ----------------------------------------------------------

const UNION_SCHEMA = `
schema Created {
  who:    String
  amount: u64
}

schema Deleted {
  who:    String
  reason: optional String
}

union Event @tag("type") {
  created: Created
  deleted: Deleted
}

schema Feed {
  id:     u64
  latest: Event
  log:    [Event]
}
`

function unionProject() {
  const dir = mkdtempSync(join(tmpdir(), 'seam-typegen-union-'))
  const seam = join(dir, 'feed.seam')
  writeFileSync(seam, UNION_SCHEMA, 'utf8')
  writeFileSync(join(dir, 'feed.types.ts'), generate(seam), 'utf8')
  return { dir, seam, body: readFileSync(join(dir, 'feed.types.ts'), 'utf8') }
}

const USE_UNION = `
import { Schema } from 'seam'
import type { FeedTypes, Event } from './feed.types'

const schema = Schema.load<FeedTypes>('feed.seam')
const event: Event = schema.validator('Event').validate(new Uint8Array())
`

function checkUnion(body) {
  const { dir } = unionProject()
  writeFileSync(join(dir, 'use_it.ts'), body, 'utf8')
  return tsc(dir, ['feed.types.ts', 'use_it.ts'])
}

test('a union renders as a TypeScript discriminated union', () => {
  const { body } = unionProject()
  assert.match(body, /export type Event =\n {2}\| \(Created & \{ type: 'created' \}\)\n {2}\| \(Deleted & \{ type: 'deleted' \}\)/)
})

test('the tag is intersected in, because no variant may declare it', () => {
  const { body } = unionProject()
  // `Created` carries who and amount and nothing else; the tag is the union's.
  assert.match(body, /export interface Created \{\n {2}who: string\n {2}amount: bigint\n\}/)
})

test('a union is a type like any other in the map and in a field', () => {
  const { body } = unionProject()
  assert.match(body, /^ {2}Event: Event$/m)
  assert.match(body, /^ {2}latest: Event$/m)
  assert.match(body, /^ {2}log: Event\[\]$/m)
})

test('tsc narrows on the tag', () => {
  // The whole point of emitting a discriminated union rather than a bare
  // intersection: checking the tag tells the compiler which variant this is.
  const result = checkUnion(`${USE_UNION}
if (event.type === 'created') {
  const amount: bigint = event.amount
  void amount
}
`)
  assert.ok(result.ok, result.output)
})

test('tsc refuses a variant field read without narrowing', () => {
  const result = checkUnion(`${USE_UNION}\nconst amount: bigint = event.amount\n`)
  assert.ok(!result.ok)
})

test('tsc refuses the wrong variant after narrowing', () => {
  const result = checkUnion(`${USE_UNION}
if (event.type === 'deleted') {
  const amount: bigint = event.amount
  void amount
}
`)
  assert.ok(!result.ok)
})

test('tsc refuses a tag the union does not declare', () => {
  const result = checkUnion(`${USE_UNION}\nif (event.type === 'archived') { }\n`)
  assert.ok(!result.ok)
})

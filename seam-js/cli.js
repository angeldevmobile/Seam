#!/usr/bin/env node
'use strict'

/**
 * The `seam` command, for the Node package.
 *
 * Deliberately the same subcommand, flags and messages as the Python one:
 * `seam typegen --check` should mean the same thing in either ecosystem's CI,
 * or the two halves of "no drift" would drift.
 */

const { mkdirSync, readFileSync, writeFileSync, existsSync } = require('node:fs')
const { dirname, resolve } = require('node:path')

const { generate, outputPath } = require('./typegen.js')

const USAGE = `\
usage: seam typegen [-h] [-o OUTPUT] [--check] schemas [schemas ...]

Generates TypeScript interfaces so the compiler can see the shape of a
validated payload. The generated file holds no rules, and no code: it is
types only, so deleting it costs static checking and nothing else.

positional arguments:
  schemas               .seam files

options:
  -h, --help            show this help message and exit
  -o, --output OUTPUT   output path (single schema only)
  --check               fail if the generated file is missing or out of date,
                        for CI
`

function fail(message) {
  process.stderr.write(`seam: error: ${message}\n`)
  return 2
}

function typegen({ schemas, output, check }) {
  let status = 0

  for (const schema of schemas) {
    let rendered
    try {
      rendered = generate(schema)
    } catch (e) {
      process.stderr.write(`error: ${e.message}\n`)
      status = 1
      continue
    }

    const target = output ? resolve(output) : outputPath(schema)

    if (check) {
      const current = existsSync(target) ? readFileSync(target, 'utf8') : null
      if (current !== rendered) {
        const what = current !== null ? 'is out of date' : 'is missing'
        process.stderr.write(`${target} ${what}; run \`seam typegen ${schema}\`\n`)
        status = 1
      }
      continue
    }

    mkdirSync(dirname(target), { recursive: true })
    // The bytes have to be the same on every platform or `--check` would fail
    // over a line ending, so the generator emits `\n` and nothing rewrites it.
    writeFileSync(target, rendered, 'utf8')
    process.stdout.write(`wrote ${target}\n`)
  }

  return status
}

function main(argv = process.argv.slice(2)) {
  const [command, ...rest] = argv

  if (command === undefined || command === '-h' || command === '--help') {
    process.stdout.write(USAGE)
    return command === undefined ? 2 : 0
  }
  if (command !== 'typegen') {
    return fail(`argument command: invalid choice: '${command}' (choose from 'typegen')`)
  }

  const schemas = []
  let output
  let check = false

  for (let i = 0; i < rest.length; i++) {
    const arg = rest[i]
    if (arg === '-h' || arg === '--help') {
      process.stdout.write(USAGE)
      return 0
    } else if (arg === '--check') {
      check = true
    } else if (arg === '-o' || arg === '--output') {
      output = rest[++i]
      if (output === undefined) return fail('argument -o/--output: expected one argument')
    } else if (arg.startsWith('--output=')) {
      output = arg.slice('--output='.length)
    } else if (arg.startsWith('-') && arg !== '-') {
      return fail(`unrecognized argument: ${arg}`)
    } else {
      schemas.push(arg)
    }
  }

  if (schemas.length === 0) return fail('the following arguments are required: schemas')
  if (output !== undefined && schemas.length > 1) return fail('--output takes a single schema')

  return typegen({ schemas, output, check })
}

module.exports = { main }

if (require.main === module) process.exit(main())

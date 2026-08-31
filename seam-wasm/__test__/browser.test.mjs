// The one thing Node cannot cover: that this actually loads in a browser.
//
// Everywhere else the module is read off disk. Here it is fetched and
// instantiated the way a real page does it, which is the only way to catch a
// wrong MIME type, a path that a bundler would have rewritten, or a top-level
// await that a browser refuses. It is a smoke test on purpose — the rules are
// already covered by the conformance suite, which runs the same `.wasm`.
//
// Skips when no browser is installed, so a checkout without one still runs the
// rest of the suite. CI installs Chromium and does not skip.

import { test } from 'node:test'
import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import { readFile } from 'node:fs/promises'
import { dirname, extname, join, normalize } from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = dirname(fileURLToPath(import.meta.url))
const ROOT = join(HERE, '..')

const TYPES = {
  '.js': 'text/javascript',
  '.mjs': 'text/javascript',
  // Wrong here and `instantiateStreaming` refuses the module, which is exactly
  // the class of mistake this test exists to catch.
  '.wasm': 'application/wasm',
  '.html': 'text/html',
  '.seam': 'text/plain',
}

let chromium
try {
  ;({ chromium } = await import('playwright'))
} catch {
  chromium = null
}

/** Serves the package directory, and nothing above it. */
function serve() {
  const server = createServer(async (req, res) => {
    try {
      const rel = normalize(decodeURIComponent(new URL(req.url, 'http://x').pathname)).replace(
        /^([/\\])+/,
        '',
      )
      const path = join(ROOT, rel)
      if (!path.startsWith(ROOT)) {
        res.writeHead(403).end()
        return
      }
      const body = await readFile(path)
      res.writeHead(200, { 'content-type': TYPES[extname(path)] ?? 'application/octet-stream' })
      res.end(body)
    } catch {
      res.writeHead(404).end()
    }
  })
  return new Promise((resolve) => {
    server.listen(0, '127.0.0.1', () => resolve(server))
  })
}

async function launch() {
  try {
    return await chromium.launch()
  } catch {
    return null
  }
}

test('the module loads and validates in a real browser', async (t) => {
  if (!chromium) return t.skip('playwright is not installed')
  const browser = await launch()
  if (!browser) return t.skip('no browser binary — run `npx playwright install chromium`')

  const server = await serve()
  const port = server.address().port

  try {
    const page = await browser.newPage()
    const errors = []
    page.on('pageerror', (e) => errors.push(String(e)))

    await page.goto(`http://127.0.0.1:${port}/__test__/fixture.html`)

    const result = await page.evaluate(async () => {
      const { Schema, SeamValidationError } = await import('/index.js')

      const schema = await Schema.parse('schema User { id: u64  name: String  seen: optional DateTime }')
      const bytes = new TextEncoder().encode(
        '{"id": 9007199254740993, "name": "Gabriel", "seen": "2026-08-30T12:00:00Z"}',
      )
      const out = schema.validator('User').validate(bytes.buffer)

      let refused = null
      try {
        schema.validator('User').validate('{"id": -1, "name": "Gabriel"}')
      } catch (e) {
        refused = e instanceof SeamValidationError ? e.issues.map((i) => `${i.path}:${i.code}`) : ['wrong error']
      }

      return {
        // A bigint cannot cross `page.evaluate`, so it is compared in the page.
        exact: typeof out.id === 'bigint' && out.id === 9007199254740993n,
        isDate: out.seen instanceof Date,
        name: out.name,
        refused,
      }
    })

    assert.deepEqual(errors, [], 'the page logged errors')
    assert.ok(result.exact, 'a u64 did not survive as an exact bigint in the browser')
    assert.ok(result.isDate, 'a DateTime did not come back as a Date')
    assert.equal(result.name, 'Gabriel')
    assert.deepEqual(result.refused, ['id:out_of_range'])
  } finally {
    server.close()
    await browser.close()
  }
})

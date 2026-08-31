// What the browser actually downloads, measured rather than estimated.
//
// The README publishes these numbers, so they are produced by a script that
// anyone can rerun instead of being typed in by hand and left to rot.

import { gzipSync, brotliCompressSync, constants } from 'node:zlib'
import { readFileSync, statSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = dirname(fileURLToPath(import.meta.url))
const WASM = join(HERE, '..', 'wasm', 'seam_wasm_bg.wasm')
const GLUE = join(HERE, '..', 'wasm', 'seam_wasm.js')
const WRAPPER = join(HERE, '..', 'index.js')

function kib(n) {
  return `${(n / 1024).toFixed(1)} KiB`
}

function row(label, bytes) {
  const gz = gzipSync(bytes, { level: 9 }).length
  const br = brotliCompressSync(bytes, {
    params: { [constants.BROTLI_PARAM_QUALITY]: 11 },
  }).length
  return {
    file: label,
    raw: kib(bytes.length),
    gzip: kib(gz),
    brotli: kib(br),
    _gz: gz,
    _br: br,
    _raw: bytes.length,
  }
}

for (const path of [WASM, GLUE, WRAPPER]) {
  try {
    statSync(path)
  } catch {
    console.error(`missing ${path} — run \`npm run build\` first`)
    process.exit(1)
  }
}

const parts = [
  row('seam_wasm_bg.wasm', readFileSync(WASM)),
  row('seam_wasm.js (glue)', readFileSync(GLUE)),
  row('index.js (wrapper)', readFileSync(WRAPPER)),
]

const total = {
  file: 'total',
  raw: kib(parts.reduce((n, p) => n + p._raw, 0)),
  gzip: kib(parts.reduce((n, p) => n + p._gz, 0)),
  brotli: kib(parts.reduce((n, p) => n + p._br, 0)),
}

console.table([...parts, total].map(({ file, raw, gzip, brotli }) => ({ file, raw, gzip, brotli })))

// Does the *published artifact* work, outside this repository?
//
// Every other test in this project runs inside the checkout, where
// `../../conformance` exists, the native module sits where the build left it,
// and every path resolves because the source tree is right there. A package
// installed from a registry has none of that, and the failures that follow are
// the ones that ruin a release: a file missing from `files` or `include`, a
// dependency nobody declared, a path that only ever worked in a checkout.
//
// So this builds each package the way the registry would receive it, installs
// it into a scratch directory **outside the repo**, and does one real piece of
// work through the public API. Not the conformance suite — that is already
// covered, and it needs files a package does not ship. Just: does it install,
// import, and validate.
//
// Publishing cannot be undone. crates.io never deletes, and on npm and PyPI the
// version number is burnt either way, so this runs before a release rather
// than after one.
//
//     node scripts/packaging-smoke.mjs

import { execFileSync } from 'node:child_process'
import { mkdtempSync, rmSync, writeFileSync, readFileSync, readdirSync, existsSync, copyFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const IS_WINDOWS = process.platform === 'win32'

const results = []
let failed = false

function run(cmd, args, opts = {}) {
  return execFileSync(cmd, args, {
    encoding: 'utf8',
    stdio: 'pipe',
    windowsHide: true,
    ...opts,
  })
}

/**
 * The interpreter to build the wheel and seed the clean venv with.
 *
 * A checkout usually has one in `.venv`, and on Windows a bare `python` is as
 * likely to be the Store alias as a real interpreter. `SEAM_PYTHON` overrides.
 */
function python() {
  if (process.env.SEAM_PYTHON) return process.env.SEAM_PYTHON
  const local = join(ROOT, '.venv', IS_WINDOWS ? 'Scripts/python.exe' : 'bin/python')
  if (existsSync(local)) return local
  return IS_WINDOWS ? 'python' : 'python3'
}

function scratch(label) {
  // Outside the repo on purpose: inside it, a stray relative path would
  // resolve and the test would pass for the wrong reason.
  const safe = label.replace(/[^a-z0-9]+/gi, '-')
  return mkdtempSync(join(tmpdir(), `seam-smoke-${safe}-`))
}

async function check(name, fn) {
  const dir = scratch(name)
  try {
    const detail = await fn(dir)
    results.push({ name, ok: true, detail })
  } catch (e) {
    failed = true
    const output = [e.stdout, e.stderr, e.message].filter(Boolean).join('\n')
    results.push({ name, ok: false, detail: output.trim().split('\n').slice(-12).join('\n') })
  } finally {
    try {
      rmSync(dir, { recursive: true, force: true })
    } catch {
      // A locked file on Windows is not worth failing a smoke test over.
    }
  }
}

//   Rust                       -

await check('crates.io / seam-core', async () => {
  // `cargo package` builds the .crate and then compiles it from the unpacked
  // copy, which is exactly the question: does it build from what ships?
  run('cargo', ['package', '--manifest-path', join(ROOT, 'seam-core/Cargo.toml'), '--allow-dirty'], {
    cwd: ROOT,
  })
  const out = run(
    'cargo',
    ['package', '--manifest-path', join(ROOT, 'seam-core/Cargo.toml'), '--allow-dirty', '--list'],
    { cwd: ROOT },
  )
  const files = out.trim().split(/\r?\n/)
  if (!files.includes('README.md')) throw new Error('README.md is not in the crate')
  if (!files.includes('src/lib.rs')) throw new Error('src/lib.rs is not in the crate')
  return `${files.length} files, compiles from the packaged copy`
})

//   Python                      --

await check('PyPI / seam-schema', async (dir) => {
  const wheels = join(dir, 'wheels')
  // `-m maturin`, not the bare command: it is usually installed into the
  // checkout's virtualenv rather than onto PATH.
  run(
    python(),
    ['-m', 'maturin', 'build', '--release', '--out', wheels,
     '--manifest-path', join(ROOT, 'seam-py/Cargo.toml')],
    { cwd: ROOT },
  )
  const wheel = readdirSync(wheels).find((f) => f.endsWith('.whl'))
  if (!wheel) throw new Error('maturin produced no wheel')

  // A brand-new interpreter environment: nothing from this repo on the path.
  const venv = join(dir, 'venv')
  run(python(), ['-m', 'venv', venv])
  const py = join(venv, IS_WINDOWS ? 'Scripts/python.exe' : 'bin/python')
  run(py, ['-m', 'pip', 'install', '--quiet', join(wheels, wheel)])

  const script = join(dir, 'use_it.py')
  writeFileSync(
    script,
    [
      'from seam_schema import Schema, ValidationError',
      // The whole promise, in one line: an integer past 2^53 stays exact.
      's = Schema.parse("schema A { id: u64  mail: String @format(email) }")',
      'out = s.validate("A", b\'{"id": 9007199254740993, "mail": "a@b.co"}\')',
      'assert out["id"] == 9007199254740993, out',
      'try:',
      '    s.validate("A", {"id": 1, "mail": "nope"})',
      '    raise SystemExit("expected a validation error")',
      'except ValidationError as e:',
      '    assert e.code == "invalid_format", e.code',
      // The CLI is a separate entry point and its own way to be broken.
      'import subprocess, sys',
      'r = subprocess.run([sys.executable, "-m", "seam_schema.cli", "--help"], capture_output=True)',
      'assert r.returncode == 0, r.stderr',
      'print("ok")',
    ].join('\n'),
    'utf8',
  )
  const out = run(py, [script], { cwd: dir })
  if (!out.includes('ok')) throw new Error(out)
  return `${wheel}, installed into a clean venv`
})

//   Node                       -

// What npm actually receives is not what a checkout contains. `napi
// pre-publish` writes `optionalDependencies` into `package.json` and moves each
// binary into its per-platform package, and only then does `npm publish` run.
// So the published `seam-schema` carries no binary of its own: `native.js`
// falls through to requiring `seam-schema-<triple>`.
//
// Packing the checkout as-is tests none of that. It passes on a `.node` sitting
// next to `index.js` and a `package.json` with no optional dependencies at all
// -- a shape no user ever installs. That is the same blind spot that let 0.1.0
// out: a gate looking at something adjacent to the artefact.
//
// So the package is staged the way the release stages it, and the fallback is
// what gets exercised.
function stageNpmPackage(js) {
  const npmDir = join(js, 'npm')
  if (!existsSync(npmDir)) run('npx', ['napi', 'create-npm-dirs'], { cwd: js, shell: IS_WINDOWS })

  // The per-platform package this host would resolve to, found the way npm
  // finds it, rather than by re-deriving napi's triple naming here.
  const triple = readdirSync(npmDir).find((t) => {
    const meta = JSON.parse(readFileSync(join(npmDir, t, 'package.json'), 'utf8'))
    return meta.os?.includes(process.platform) && meta.cpu?.includes(process.arch)
  })
  if (!triple) throw new Error(`no per-platform package targets ${process.platform}-${process.arch}`)

  const binary = JSON.parse(readFileSync(join(npmDir, triple, 'package.json'), 'utf8')).main
  if (!existsSync(join(npmDir, triple, binary))) {
    // The release job stages all three from downloaded artefacts and lets
    // `napi pre-publish` place them. A checkout has only the one it just
    // built, so place that one.
    if (!existsSync(join(js, binary))) {
      throw new Error(`no ${binary} in seam-js or npm/${triple}: run \`npm run build\` there first`)
    }
    copyFileSync(join(js, binary), join(npmDir, triple, binary))
  }

  // Same reason: after `pre-publish` these are already written. Without it
  // they are not, and packing now would produce a package that cannot resolve
  // a binary at all.
  const pkgPath = join(js, 'package.json')
  const original = readFileSync(pkgPath, 'utf8')
  const pkg = JSON.parse(original)
  let restore = null
  if (!pkg.optionalDependencies) {
    pkg.optionalDependencies = Object.fromEntries(
      readdirSync(npmDir).map((t) => [
        JSON.parse(readFileSync(join(npmDir, t, 'package.json'), 'utf8')).name,
        pkg.version,
      ]),
    )
    writeFileSync(pkgPath, `${JSON.stringify(pkg, null, 2)}\n`, 'utf8')
    restore = () => writeFileSync(pkgPath, original, 'utf8')
  }

  return { triple, binary, platformDir: join(npmDir, triple), restore }
}

await check('npm / seam-schema', async (dir) => {
  const js = join(ROOT, 'seam-js')
  const { triple, binary, platformDir, restore } = stageNpmPackage(js)

  let tarball
  let platformTarball
  try {
    const packedPlatform = run('npm', ['pack', '--pack-destination', dir], {
      cwd: platformDir,
      shell: IS_WINDOWS,
    })
    platformTarball = join(dir, packedPlatform.trim().split(/\r?\n/).pop())

    const packed = run('npm', ['pack', '--pack-destination', dir], { cwd: js, shell: IS_WINDOWS })
    tarball = packed.trim().split(/\r?\n/).pop()
  } finally {
    if (restore) restore()
  }

  // `overrides` because the version under test is by definition not on the
  // registry yet, and an optional dependency npm cannot fetch is one it skips
  // in silence -- the install would succeed and `require` would throw, which
  // is precisely the 0.1.0 failure. Only the main tarball is installed:
  // passing the platform one too would make it a direct dependency, which npm
  // rejects as EOVERRIDE and which is an install shape no consumer has.
  writeFileSync(
    join(dir, 'package.json'),
    JSON.stringify({
      name: 'consumer',
      private: true,
      overrides: { [`seam-schema-${triple}`]: `file:${platformTarball}` },
    }),
    'utf8',
  )
  run('npm', ['install', '--no-audit', '--no-fund', join(dir, tarball)], {
    cwd: dir,
    shell: IS_WINDOWS,
  })

  const strays = readdirSync(join(dir, 'node_modules', 'seam-schema')).filter((f) => f.endsWith('.node'))
  if (strays.length) throw new Error(`seam-schema ships binaries of its own again: ${strays.join(', ')}`)
  if (!existsSync(join(dir, 'node_modules', `seam-schema-${triple}`, binary))) {
    throw new Error(`the ${triple} package did not provide ${binary}`)
  }

  const script = join(dir, 'use_it.cjs')
  writeFileSync(
    script,
    [
      "const { Schema, SeamValidationError } = require('seam-schema')",
      "const s = Schema.parse('schema A { id: u64  mail: String @format(email) }')",
      'const out = s.validate("A", Buffer.from(\'{"id": 9007199254740993, "mail": "a@b.co"}\'))',
      "if (typeof out.id !== 'bigint' || out.id !== 9007199254740993n) throw new Error('u64 lost: ' + out.id)",
      'try {',
      "  s.validate('A', Buffer.from('{\"id\":1,\"mail\":\"nope\"}'))",
      "  throw new Error('expected a validation error')",
      '} catch (e) {',
      '  if (!(e instanceof SeamValidationError)) throw e',
      "  if (e.code !== 'invalid_format') throw new Error('wrong code: ' + e.code)",
      '}',
      "console.log('ok')",
    ].join('\n'),
    'utf8',
  )
  const out = run(process.execPath, [script], { cwd: dir })
  if (!out.includes('ok')) throw new Error(out)
  return `${tarball}, installed into a clean project`
})

//   Browser build                    -

await check('npm / seam-schema-wasm', async (dir) => {
  const wasmDir = join(ROOT, 'seam-wasm')
  if (!existsSync(join(wasmDir, 'wasm/seam_wasm_bg.wasm'))) {
    throw new Error('run `npm run build` in seam-wasm first')
  }
  const packed = run('npm', ['pack', '--pack-destination', dir], { cwd: wasmDir, shell: IS_WINDOWS })
  const tarball = packed.trim().split(/\r?\n/).pop()

  writeFileSync(
    join(dir, 'package.json'),
    JSON.stringify({ name: 'consumer', private: true, type: 'module' }),
    'utf8',
  )
  run('npm', ['install', '--no-audit', '--no-fund', join(dir, tarball)], {
    cwd: dir,
    shell: IS_WINDOWS,
  })

  const script = join(dir, 'use_it.mjs')
  writeFileSync(
    script,
    [
      "import { Schema, SeamValidationError } from 'seam-schema-wasm'",
      "const s = await Schema.parse('schema A { id: u64  mail: String @format(email) }')",
      'const out = s.validate("A", \'{"id": 9007199254740993, "mail": "a@b.co"}\')',
      "if (out.id !== 9007199254740993n) throw new Error('u64 lost: ' + out.id)",
      'try {',
      '  s.validate("A", \'{"id":1,"mail":"nope"}\')',
      "  throw new Error('expected a validation error')",
      '} catch (e) {',
      '  if (!(e instanceof SeamValidationError)) throw e',
      "  if (e.code !== 'invalid_format') throw new Error('wrong code: ' + e.code)",
      '}',
      "console.log('ok')",
    ].join('\n'),
    'utf8',
  )
  const out = run(process.execPath, [script], { cwd: dir })
  if (!out.includes('ok')) throw new Error(out)
  return `${tarball}, imported as ESM from a clean project`
})

//   report                      --

console.log()
for (const { name, ok, detail } of results) {
  console.log(`${ok ? 'ok  ' : 'FAIL'}  ${name}`)
  console.log(`      ${detail.replace(/\n/g, '\n      ')}`)
}
console.log()

if (failed) {
  console.error('A package does not work once installed. Do not publish.')
  process.exit(1)
}
console.log('Every package installs and validates from outside the repository.')

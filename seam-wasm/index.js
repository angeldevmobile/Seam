/**
 * The idiomatic surface over the generated wasm bindings.
 *
 * Deliberately the same shape as the Node package: same class names, same
 * `issues`/`path`/`code`, same `SeamValidationError`. Code that moves between a
 * service and a browser should be re-imported, not rewritten.
 *
 * Two differences, both forced by the medium rather than chosen:
 *
 *   1. **This package takes bytes, not objects.** By the time `JSON.parse` has
 *      run, an integer past 2^53 is already the wrong number. Accepting a
 *      parsed object in the browser would offer, in the runtime where the
 *      problem is worst, the exact path Seam exists to avoid.
 *   2. **`Schema.parse` is async.** WebAssembly of this size cannot be
 *      instantiated synchronously on a browser's main thread, so the module is
 *      compiled on first use. It is one `await`, in the place you were already
 *      awaiting the fetch of the `.seam` file.
 */

import init, { Schema as WasmSchema } from './wasm/seam_wasm.js'

const encoder = new TextEncoder()

/**
 * Beyond this, `validate` refuses before copying anything into the module.
 *
 * WebAssembly memory grows and is never returned to the host, so one hostile
 * payload can raise a tab's floor for the rest of its life. The engine's own
 * limits bound nesting, item counts, key counts and string length, but nothing
 * bounds the document as a whole — and this is the only binding where that
 * cost is permanent. Raise it with `maxBytes` when a legitimate request really
 * is larger.
 */
export const DEFAULT_MAX_BYTES = 8 * 1024 * 1024

let booted

/**
 * Compiles the wasm module. Idempotent, and called for you by `Schema.parse`.
 *
 * Pass `source` to control where the bytes come from — a `Response`, an
 * `ArrayBuffer`, a `URL`, or an already-compiled `WebAssembly.Module`. Without
 * it, the module beside this file is used, read from disk under Node and
 * fetched in a browser.
 */
export function ready(source) {
  if (booted === undefined) booted = boot(source)
  return booted
}

async function boot(source) {
  if (source !== undefined) return init({ module_or_path: source })

  const url = new URL('./wasm/seam_wasm_bg.wasm', import.meta.url)

  // Node reads it off disk: `fetch` does not take a file: URL, and a bundler
  // will have rewritten this path long before it ever runs in a browser.
  const isNode =
    typeof process !== 'undefined' && process.versions != null && process.versions.node != null
  if (isNode && url.protocol === 'file:') {
    const { readFile } = await import('node:fs/promises')
    return init({ module_or_path: await readFile(url) })
  }
  return init({ module_or_path: url })
}

/**
 * Raised when a payload does not satisfy its schema.
 *
 * Nothing here is built when the error is thrown. The engine hands over the
 * issues and the message is produced the first time it is read, so a caller
 * that lets the error propagate pays for none of it.
 */
export class SeamValidationError extends Error {
  #message

  constructor(issues) {
    super()
    this.name = 'SeamValidationError'
    this.issues = issues
  }

  get message() {
    if (this.#message === undefined) {
      const [first, ...rest] = this.issues
      this.#message = first
        ? `${first.path}: ${first.message} (${first.code})` +
          (rest.length ? `, and ${rest.length} more` : '')
        : 'validation failed'
    }
    return this.#message
  }

  /** The first issue's location, for the common case of reading one. */
  get path() {
    return this.issues[0]?.path ?? ''
  }

  get code() {
    return this.issues[0]?.code ?? ''
  }
}

/** Anything byte-shaped becomes bytes; anything else is refused by name. */
function bytesOf(payload) {
  if (typeof payload === 'string') return encoder.encode(payload)
  if (payload instanceof Uint8Array) return payload
  if (payload instanceof ArrayBuffer) return new Uint8Array(payload)
  if (ArrayBuffer.isView(payload)) {
    return new Uint8Array(payload.buffer, payload.byteOffset, payload.byteLength)
  }

  // A plain object is the one thing this package must not accept: it has
  // already been through `JSON.parse`, so any 64-bit integer in it is already
  // wrong. Say so, rather than validate a value nobody can trust.
  const what = payload === null ? 'null' : typeof payload
  throw new TypeError(
    `seam: validate expects JSON bytes or text, received ${what}. ` +
      'In the browser Seam parses the JSON itself, because JSON.parse corrupts ' +
      'any integer past 2^53 before a validator could see it. Pass the response ' +
      'body itself: await res.arrayBuffer(), or await res.text().',
  )
}

export class Validator {
  #inner
  #maxBytes

  constructor(inner, maxBytes) {
    this.#inner = inner
    this.#maxBytes = maxBytes
  }

  get typeName() {
    return this.#inner.typeName
  }

  /**
   * Validates raw JSON: a `Uint8Array`, an `ArrayBuffer`, any typed array, or
   * a string.
   *
   * @throws {SeamValidationError} when the payload does not satisfy the schema
   * @throws {TypeError} when it is not JSON bytes or text
   * @throws {RangeError} when it is larger than `maxBytes`
   */
  validate(payload) {
    const bytes = bytesOf(payload)
    if (bytes.length > this.#maxBytes) {
      throw new RangeError(
        `seam: payload is ${bytes.length} bytes, over the limit of ${this.#maxBytes}. ` +
          'Raise it with `maxBytes` if a legitimate request really is this large.',
      )
    }
    const outcome = this.#inner.validate(bytes)
    if (outcome.ok) return outcome.value
    throw new SeamValidationError(outcome.issues)
  }
}

export class Schema {
  #inner
  #maxBytes

  constructor(inner, maxBytes) {
    this.#inner = inner
    this.#maxBytes = maxBytes
  }

  /**
   * Compiles a `.seam` source. Instantiates the wasm module on first use.
   *
   * There is no `load`: a browser has no filesystem, and taking a URL here
   * would fold fetching into validation. Fetch the file yourself, or let your
   * bundler inline it, and pass the text.
   */
  static async parse(source, options) {
    const maxBytes = options?.maxBytes ?? DEFAULT_MAX_BYTES
    if (!Number.isInteger(maxBytes) || maxBytes < 1) {
      throw new RangeError('seam: `maxBytes` must be a positive integer')
    }
    await ready(options?.wasm)
    return new Schema(WasmSchema.parse(source), maxBytes)
  }

  typeNames() {
    return this.#inner.typeNames()
  }

  /**
   * Binds one type for repeated validation. Everything that does not depend on
   * the payload is resolved here rather than on every call.
   */
  validator(typeName, limits) {
    // Same shape as `validate`, for the same reason: a name the schema does
    // not declare is `unknown_type`, a code the mapping spec fixes. Reporting
    // it as a generic failure would give one mistake a different code in every
    // binding, which is the drift this project exists to prevent.
    const outcome = this.#inner.validator(typeName, limits)
    if (!outcome.ok) throw new SeamValidationError(outcome.issues)
    return new Validator(outcome.value, limits?.maxBytes ?? this.#maxBytes)
  }

  /** Convenience for one-off validation; binds and discards. */
  validate(typeName, payload, limits) {
    return this.validator(typeName, limits).validate(payload)
  }
}

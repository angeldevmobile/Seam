'use strict'

const native = require('./native.js')

/**
 * Raised when a payload does not satisfy its schema.
 *
 * Nothing here is built when the error is thrown. The engine hands over the
 * issues and the message is produced the first time it is read, so a caller
 * that lets the error propagate pays for none of it.
 */
class SeamValidationError extends Error {
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

class Validator {
  #inner

  constructor(inner) {
    this.#inner = inner
  }

  get typeName() {
    return this.#inner.typeName
  }

  /**
   * Validates a plain object, or raw JSON as a `Buffer`, `Uint8Array` or
   * `string`.
   *
   * Hand it the bytes when you have them. `JSON.parse` turns any integer past
   * 2^53 into the wrong number before a validator could ever see it, so Seam
   * reads the JSON itself; a value that arrived through `JSON.parse` is
   * already whatever `JSON.parse` made of it.
   *
   * @throws {SeamValidationError}
   */
  validate(payload) {
    const outcome = this.#inner.validate(payload)
    if (outcome.ok) return outcome.value
    throw new SeamValidationError(outcome.issues)
  }
}

class Schema {
  #inner

  constructor(inner) {
    this.#inner = inner
  }

  static parse(source) {
    return new Schema(native.Schema.parse(source))
  }

  static load(path) {
    return new Schema(native.Schema.load(path))
  }

  typeNames() {
    return this.#inner.typeNames()
  }

  /**
   * The schema as plain data, for tooling that generates types.
   *
   * Deliberately not a validator: it carries shape, not rules-as-behaviour.
   * `seam typegen` is the only caller in this package, and it is written here
   * rather than in Rust so that the language's own types are chosen by the
   * binding that speaks it.
   */
  describe() {
    return this.#inner.describe()
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
    if (outcome.ok) return new Validator(outcome.value)
    throw new SeamValidationError(outcome.issues)
  }

  /** Convenience for one-off validation; binds and discards. */
  validate(typeName, payload, limits) {
    return this.validator(typeName, limits).validate(payload)
  }
}

module.exports = { Schema, Validator, SeamValidationError }

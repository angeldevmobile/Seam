/** One failure, with a stable `path` and `code`. `message` is for humans. */
export interface Issue {
  path: string
  code: string
  message: string
}

/** Bounds on hostile input. Omitted fields keep the engine's defaults. */
export interface Limits {
  maxDepth?: number
  maxItems?: number
  maxStringBytes?: number
  maxObjectKeys?: number
}

/**
 * Raised when a payload does not satisfy its schema.
 *
 * `message` is the summary, as JavaScript expects of an `Error`. The first
 * issue's own message is `issues[0].message`.
 */
export declare class SeamValidationError extends Error {
  readonly issues: Issue[]
  readonly path: string
  readonly code: string
}

export declare class Validator {
  readonly typeName: string
  /**
   * Validates a plain object, or raw JSON as a `Buffer`, `Uint8Array` or
   * `string`. Prefer the bytes: `JSON.parse` corrupts any integer past 2^53
   * before a validator could see it, so Seam reads the JSON itself.
   *
   * @throws {SeamValidationError}
   */
  validate(payload: unknown): unknown
}

export declare class Schema {
  static parse(source: string): Schema
  static load(path: string): Schema
  typeNames(): string[]
  validator(typeName: string, limits?: Limits): Validator
  validate(typeName: string, payload: unknown, limits?: Limits): unknown
}

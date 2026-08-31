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

// --------------------------------------------------------- the schema as data

/** One type as `describe` reports it. */
export type TypeDescription =
  | { kind: 'bool' | 'float' | 'string' | 'date' | 'datetime' }
  /** `fitsJsNumber` is false for `i64` and `u64`: those are `bigint`. */
  | { kind: 'int'; name: string; signed: boolean; fitsJsNumber: boolean }
  | { kind: 'enum'; values: string[] }
  | { kind: 'array'; item: TypeDescription; itemNullable: boolean }
  | { kind: 'object'; object: ObjectDescription }
  | { kind: 'ref'; name: string }

/** Rule names keep their `.seam` spelling; they are schema values, not keys. */
export type RuleDescription =
  | { rule: 'min_len' | 'max_len' | 'min_items' | 'max_items'; value: number }
  /** `bigint`, because a `u64` bound does not fit a `number`. */
  | { rule: 'range'; min: bigint; max: bigint }

export interface FieldDescription {
  name: string
  type: TypeDescription
  /** Absence and nullability are two axes. A field may be either, or both. */
  optional: boolean
  nullable: boolean
  rules: RuleDescription[]
}

export interface ObjectDescription {
  name: string
  denyUnknownFields: boolean
  /** Declaration order: it is the order errors are reported in. */
  fields: FieldDescription[]
}

/** Every declared type, keyed by name. */
export type SchemaDescription = Record<string, ObjectDescription>

// ------------------------------------------------------------- the API proper

/**
 * A bound type. `T` is what `validate` returns, which `seam typegen` fills in
 * when the schema was loaded with a generated type map.
 */
export declare class Validator<T = unknown> {
  readonly typeName: string
  /**
   * Validates a plain object, or raw JSON as a `Buffer`, `Uint8Array` or
   * `string`. Prefer the bytes: `JSON.parse` corrupts any integer past 2^53
   * before a validator could see it, so Seam reads the JSON itself.
   *
   * @throws {SeamValidationError}
   */
  validate(payload: unknown): T
}

/**
 * A parsed schema.
 *
 * `M` maps each declared type name to its shape. `seam typegen` generates one:
 *
 * ```ts
 * import { Schema } from 'seam-schema'
 * import type { UserTypes } from './user.types'
 *
 * const schema = Schema.load<UserTypes>('user.seam')
 * const user = schema.validator('User').validate(bytes) // typed as User
 * ```
 *
 * Without it everything still works and `validate` returns `unknown`, which is
 * the honest type for a payload nothing has described.
 */
export declare class Schema<M = Record<string, unknown>> {
  static parse<M = Record<string, unknown>>(source: string): Schema<M>
  static load<M = Record<string, unknown>>(path: string): Schema<M>
  typeNames(): string[]
  /**
   * The schema as plain data, for tooling that generates types. Shape, not
   * rules-as-behaviour.
   */
  describe(): SchemaDescription
  /** Binds one type. A name the schema does not declare is a compile error. */
  validator<K extends keyof M & string>(typeName: K, limits?: Limits): Validator<M[K]>
  /** Convenience for one-off validation; binds and discards. */
  validate<K extends keyof M & string>(typeName: K, payload: unknown, limits?: Limits): M[K]
}

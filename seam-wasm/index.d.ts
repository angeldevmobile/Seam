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
  /**
   * The largest payload `validate` will copy into the module, in bytes.
   * Defaults to {@link DEFAULT_MAX_BYTES}.
   *
   * Specific to this package: wasm memory grows and is never returned to the
   * host, so one oversized payload raises a tab's floor for the rest of its
   * life. The engine's other limits bound the shape of a document; this one
   * bounds the document.
   */
  maxBytes?: number
}

export interface SchemaOptions {
  /** Default `maxBytes` for every validator this schema hands out. */
  maxBytes?: number
  /** Where the wasm module comes from, if not the file beside this one. */
  wasm?: Response | ArrayBuffer | Uint8Array | URL | WebAssembly.Module
}

/** 8 MiB. See {@link Limits.maxBytes}. */
export declare const DEFAULT_MAX_BYTES: number

/**
 * Compiles the wasm module. Idempotent, and called for you by `Schema.parse`.
 *
 * Await it at startup if you would rather pay the cost before the first
 * request than during it.
 */
export declare function ready(
  source?: Response | ArrayBuffer | Uint8Array | URL | WebAssembly.Module,
): Promise<unknown>

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

/** JSON, as bytes or text. Never an already-parsed object — see `validate`. */
export type Payload = Uint8Array | ArrayBuffer | ArrayBufferView | string

/**
 * A bound type. `T` is what `validate` returns, which `seam typegen` fills in
 * when the schema was parsed with a generated type map.
 */
export declare class Validator<T = unknown> {
  readonly typeName: string
  /**
   * Validates raw JSON.
   *
   * There is no object path in this package, on purpose: `JSON.parse` corrupts
   * any integer past 2^53 before a validator could see it, so in the browser
   * Seam reads the bytes itself. Pass the response body.
   *
   * @throws {SeamValidationError} when the payload does not satisfy the schema
   * @throws {TypeError} when it is not JSON bytes or text
   * @throws {RangeError} when it is larger than `maxBytes`
   */
  validate(payload: Payload): T
}

/**
 * A parsed schema.
 *
 * `M` maps each declared type name to its shape. `seam typegen` generates one,
 * and the same generated file serves this package and the Node one:
 *
 * ```ts
 * import { Schema } from 'seam-schema-wasm'
 * import type { UserTypes } from './contracts/user.types'
 *
 * const schema = await Schema.parse<UserTypes>(await (await fetch('/user.seam')).text())
 * const user = schema.validator('User').validate(await res.arrayBuffer())
 * ```
 *
 * Without it everything still works and `validate` returns `unknown`, which is
 * the honest type for a payload nothing has described.
 */
export declare class Schema<M = Record<string, unknown>> {
  /**
   * Compiles a `.seam` source, instantiating the wasm module on first use.
   *
   * Async because WebAssembly of this size cannot be compiled synchronously on
   * a browser's main thread. It is one `await`, where you were already
   * awaiting the fetch of the `.seam` file.
   *
   * There is no `load`: a browser has no filesystem, and taking a URL here
   * would fold fetching into validation.
   */
  static parse<M = Record<string, unknown>>(
    source: string,
    options?: SchemaOptions,
  ): Promise<Schema<M>>
  typeNames(): string[]
  /** Binds one type. A name the schema does not declare is a compile error. */
  validator<K extends keyof M & string>(typeName: K, limits?: Limits): Validator<M[K]>
  /** Convenience for one-off validation; binds and discards. */
  validate<K extends keyof M & string>(typeName: K, payload: Payload, limits?: Limits): M[K]
}

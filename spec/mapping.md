# Seam mapping specification

Normative. Every rule here is a promise a binding must keep, and every rule has
a case in [`conformance/`](../conformance/). Where this document and an
implementation disagree, the implementation is wrong.

Status: **draft**. Nothing is frozen until 1.0.

---

## 1. Presence

Absence and nullability are independent axes. A binding that collapses them is
non-conformant.

| `.seam` | Absent key | Explicit null |
|---|---|---|
| `T` | error `required` | error `null_not_allowed` |
| `T?` | error `required` | accepted |
| `optional T` | accepted | error `null_not_allowed` |
| `optional T?` | accepted | accepted |

Host representation:

| `.seam` | Rust | Python | TypeScript | Java |
|---|---|---|---|---|
| `T` | `T` | `T` | `T` | `T` |
| `T?` | `Option<T>` | `T \| None` | `T \| null` | `@Nullable T` |
| `optional T` | `Option<T>` | `T \| Absent` | `T \| undefined` | `Optional<T>` |
| `optional T?` | `Option<Option<T>>` | `T \| None \| Absent` | `T \| null \| undefined` | `JsonNullable<T>` |

`Absent` is a Seam-provided sentinel, not `None`. A binding must not use the
host's null to represent absence.

**Serialization.** A field that validated as absent must not be emitted. This is
the rule that makes PATCH work: absent means *leave the stored value alone*,
null means *set it to null*.

## 2. Integers

Width and signedness are part of the type. Range is checked against the declared
width, not the host's.

| `.seam` | Range | Rust | Python | TypeScript | Java |
|---|---|---|---|---|---|
| `i8`/`i16`/`i32` | signed, per width | same | `int` | `number` | `byte`/`short`/`int` |
| `i64` | −2⁶³ … 2⁶³−1 | `i64` | `int` | `bigint` | `long` |
| `u8`/`u16`/`u32` | unsigned, per width | same | `int` | `number` | `short`/`int`/`long` |
| `u64` | 0 … 2⁶⁴−1 | `u64` | `int` | `bigint` | `long`, range-checked |

**No integer may pass through a float.** A binding that parses JSON into a
double before handing it to the core is non-conformant, because everything above
2⁵³ is already corrupted by then.

**64-bit integers are `bigint` in JavaScript, never `number`.** `u64` exceeds
`Long.MAX_VALUE` in Java, so the JVM binding range-checks on the way out and
reports `out_of_range` rather than wrapping.

Integers wider than 64 bits are rejected with `integer_too_wide`.

## 3. Floats

`f64` only. `NaN`, `+Infinity` and `−Infinity` are rejected with `not_finite`;
none of them survive JSON, so admitting them would make round-tripping a lie.

An integer where a `f64` is declared is accepted. The reverse is not.

## 4. Dates and times

| `.seam` | Wire form | Rust | Python | TypeScript | Java |
|---|---|---|---|---|---|
| `Date` | `YYYY-MM-DD` | `NaiveDate` | `datetime.date` | `string` | `LocalDate` |
| `DateTime` | RFC 3339 with offset | `DateTime<Utc>` | aware `datetime` | `Date` | `Instant` |

Two rules, both deliberate:

1. **A `DateTime` without an offset is `missing_timezone`.** Not a warning, not a
   local-time assumption. "Local" is different on each side of a boundary, so
   guessing produces a value that is wrong somewhere.
2. **A `Date` never becomes a JavaScript `Date`.** JS has no date-only type, and
   pushing a calendar date through an instant is what produces off-by-one-day
   bugs. The JS binding hands back the ISO string.

Python note: `datetime` subclasses `date`, so a binding must check
`type(v) is date`, not `isinstance(v, date)`.

Accepted: `Z`, `z`, `±HH:MM`, optional fractional seconds, second value `60`
(leap second). Rejected: single-digit components, a space instead of `T`,
`±H:MM`.

## 5. Strings

UTF-8. `@min_len` and `@max_len` count **Unicode scalar values**, not bytes and
not UTF-16 code units — so a string's length is the same number in every
binding, which is the only property that makes the constraint portable.

## 6. Objects and arrays

Nesting is arbitrarily deep, bounded by `max_depth`.

Unknown keys are rejected by default. At a boundary an unexpected key is more
often a stale client than a helpful one.

Object key order does not affect validation. Error order is stable: fields in
declaration order, then unknown keys in sorted order, depth first.

## 7. Errors

`path` and `code` are stable API. `message` is for humans and may be reworded in
any release.

`path` is a sequence of keys and indices from the root; the empty path is the
root. Rendered form: `user.tags[2]`.

Validation reports every issue in one pass, never just the first.

Codes: `required`, `null_not_allowed`, `type_mismatch`, `out_of_range`,
`integer_too_wide`, `not_finite`, `too_short`, `too_long`, `too_few_items`,
`too_many_items`, `not_in_enum`, `invalid_date`, `invalid_datetime`,
`missing_timezone`, `unknown_field`, `depth_exceeded`, `size_exceeded`,
`unknown_type`.

Adding a code is a minor change. Renaming one, removing one, or changing which
condition produces one is breaking.

## 8. Limits

Enforced in the core so bindings inherit them.

| Limit | Default |
|---|---|
| `max_depth` | 64 |
| `max_items` | 10 000 |
| `max_string_bytes` | 1 MiB |
| `max_object_keys` | 1 000 |

These are defaults, not a security boundary. A service taking untrusted input
should set them from what a legitimate request actually looks like.

## Open questions

- Unicode normalization: should `@min_len` count before or after NFC?
- Should `u64` in the JVM binding be `long` with a range check, or `BigInteger`?
- Does `optional T?` deserve distinct sentinels in Python, or is a single
  `Absent` enough?

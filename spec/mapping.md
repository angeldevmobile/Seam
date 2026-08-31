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

**In JavaScript, `undefined` is absence.** A property holding `undefined` reads
as not sent, which is already what `JSON.stringify` does with it; a property
holding `null` is null. The three states the language has land on the two axes
without needing a sentinel at all, which is the one place JavaScript is better
shaped for this than Python.

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

**An integer the host cannot hold exactly is rejected with `unsafe_integer`.**
A JavaScript `number` past 2^53 is already not the value that was sent, and no
check afterwards recovers it, so validating it would bless a wrong answer. This
never arises in a language whose integers are exact, and it is why a binding
should be handed raw bytes rather than values its own parser has already been
through.

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

An array element has two states, a value or null, so only nullability applies to
it. Absence is a property of a key, and an array has no keys: a missing element
would change the length, which is a different fact.

Element nullability is therefore independent of the field's:

| `.seam` | The list | Its elements |
|---|---|---|
| `[String]` | required, non-null | non-null |
| `[String?]` | required, non-null | may be null |
| `optional [String]?` | may be absent or null | non-null |
| `optional [String?]?` | may be absent or null | may be null |

| `.seam` | Rust | Python | TypeScript | Java |
|---|---|---|---|---|
| `[String]` | `Vec<String>` | `list[str]` | `string[]` | `List<String>` |
| `[String?]` | `Vec<Option<String>>` | `list[str \| None]` | `(string \| null)[]` | `List<@Nullable String>` |

A null element where the type does not allow one is `null_not_allowed` at the
element's own path: `tags[1]`, not `tags`.

Nesting is arbitrarily deep, bounded by `max_depth`.

Unknown keys are rejected by default. At a boundary an unexpected key is more
often a stale client than a helpful one.

Object key order does not affect validation. Error order is stable: fields in
declaration order, then unknown keys in sorted order, depth first.

## 7. Tagged unions

A `union` is a choice between declared object types, told apart by the value of
one field:

```
union Event @tag("type") {
  created: Created
  deleted: Deleted
}
```

**`@tag` is mandatory.** There is no default and no inference from the
variants. A union that guessed which field decides would be guessing what the
data means, which is the same mistake as reading a naive datetime as local
time.

**The tag belongs to the union, not to its variants.** A variant that declared
the tag field itself would be a second source of truth for one value, and the
two could disagree; the parser rejects it. Three consequences follow, and a
binding must keep all three:

1. The tag is **not** an unknown field of the variant. Reporting `unknown_field`
   for it is non-conformant.
2. The tag **is** carried through to the validated value. A binding that
   returned the variant's declared fields alone would hand back an object
   missing its own discriminant.
3. A variant is **not** a level of nesting. An issue inside the chosen variant
   is reported at the union's own path — `latest.amount`, never
   `latest.created.amount`.

Reading the tag:

| Payload | Verdict |
|---|---|
| tag absent | `required`, at the tag's path |
| tag is null | `null_not_allowed`, at the tag's path |
| tag is not a string | `type_mismatch`, at the tag's path |
| tag names no variant | `unknown_variant`, at the tag's path |
| tag names a variant | the variant is validated against the same object |

**An unknown variant stops there.** It is the only issue reported, even when
the rest of the payload is also wrong. With no variant chosen there is no shape
to check the rest against, and picking one anyway would produce a list of
errors about a payload the caller never claimed to send.

A variant must name a declared `schema`, never a built-in type and never
another union: resolving a union of unions would need a second discriminant,
and nothing in the payload says which one to read first.

Unions and objects share one namespace. A name is declared once, as one or the
other, and a reference names one thing.

Host representation:

| `.seam` | Rust | Python | TypeScript |
|---|---|---|---|
| `union U @tag("t")` | `enum U` | `Union[...]` of `TypedDict`, tag pinned to `Literal` | discriminated union, tag intersected in |

Both generators pin the tag to a literal type, which is what lets a type
checker narrow on it: `if (e.type === 'created')` in TypeScript and
`if event["type"] == "created"` in Python each make the rest of the value the
created variant, checked statically.

## 8. What a binding may accept

A binding must take **raw JSON bytes**. Everything in this document about
integers depends on it: a binding handed values its host's parser has already
produced cannot keep §2, because `JSON.parse` has corrupted anything past 2⁵³
before validation begins.

A binding **may** also take its host's own values, and the Rust, Python and
Node bindings do — a Python `dict` and a JavaScript object are useful inputs
where the payload was built in-process rather than received. Such a path must
report `unsafe_integer` rather than validate a number the host already holds
inexactly.

**A binding may refuse that second path entirely.** The WebAssembly binding
does: in a browser an already-parsed object has come through `JSON.parse` in
essentially every case, so accepting one would offer the corrupting path in the
runtime where it is most likely to be taken. Refusing is conformant. Accepting
without reporting `unsafe_integer` is not.

Two further allowances, both for hosts rather than for rules:

- A binding may compile asynchronously where the host requires it. WebAssembly
  of any size cannot be instantiated synchronously on a browser's main thread,
  so `Schema.parse` returns a promise there and is synchronous elsewhere.
- A binding need not offer loading a schema from a path where the host has no
  filesystem.

Neither changes a verdict, which is the only thing this document fixes.

## 9. Errors

`path` and `code` are stable API. `message` is for humans and may be reworded in
any release.

`path` is a sequence of keys and indices from the root; the empty path is the
root. Rendered form: `user.tags[2]`.

Validation reports every issue in one pass, never just the first.

Codes: `required`, `null_not_allowed`, `type_mismatch`, `out_of_range`,
`unsafe_integer`, `integer_too_wide`, `not_finite`, `too_short`, `too_long`, `too_few_items`,
`too_many_items`, `not_in_enum`, `invalid_date`, `invalid_datetime`,
`missing_timezone`, `unknown_field`, `unknown_variant`, `depth_exceeded`,
`size_exceeded`, `unknown_type`.

Adding a code is a minor change. Renaming one, removing one, or changing which
condition produces one is breaking.

## 10. Limits

Enforced in the core so bindings inherit them.

| Limit | Default | Applies to |
|---|---|---|
| `max_depth` | 64 | any object or array, as the validator descends |
| `max_items` | 10 000 | the length of any one array |
| `max_string_bytes` | 1 MiB | any string value, including `Date`, `DateTime` and enum values |
| `max_object_keys` | 1 000 | the number of keys in any one object |

`max_string_bytes` counts **bytes**, not characters. A limit measured in
characters would not bound memory, which is the only thing it is there for.

Exceeding any of these is `size_exceeded`, except depth, which is
`depth_exceeded`.

These are defaults, not a security boundary. A service taking untrusted input
should set them from what a legitimate request actually looks like.

## Open questions

- Unicode normalization: should `@min_len` count before or after NFC?
- Should `u64` in the JVM binding be `long` with a range check, or `BigInteger`?
- Does `optional T?` deserve distinct sentinels in Python, or is a single
  `Absent` enough?

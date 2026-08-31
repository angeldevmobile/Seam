//! Tagged unions, from the `.seam` text through to the verdict.
//!
//! Parser and validator together on purpose: a union is a language feature
//! before it is a validation rule, and most of what can go wrong with one is
//! the schema saying something that cannot mean anything.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use seam_core::{
    error::Code,
    parse,
    schema::Schema,
    validate,
    value::{Int, Value},
    Limits,
};

const EVENTS: &str = r#"
schema Created {
  who:    String
  amount: u64
}

schema Deleted {
  who:    String
  reason: optional String
}

union Event @tag("type") {
  created: Created
  deleted: Deleted
}
"#;

fn schema(src: &str) -> Schema {
    parse(src).unwrap_or_else(|e| panic!("parsing failed: {e}"))
}

fn obj(pairs: &[(&str, Value)]) -> Value {
    let mut m = BTreeMap::new();
    for (k, v) in pairs {
        m.insert((*k).to_string(), v.clone());
    }
    Value::Object(m)
}

fn s(v: &str) -> Value {
    Value::String(v.into())
}

fn codes(schema: &Schema, type_name: &str, v: &Value) -> Vec<(String, Code)> {
    match validate(schema, type_name, v, Limits::DEFAULT) {
        Ok(()) => vec![],
        Err(e) => e
            .issues
            .into_iter()
            .map(|i| (i.path.render(), i.code))
            .collect(),
    }
}

fn why(src: &str) -> String {
    match parse(src) {
        Ok(_) => panic!("expected this schema to be rejected:\n{src}"),
        Err(e) => e.message,
    }
}

// --- the schema language ----------------------------------------------------

#[test]
fn a_union_declares_its_variants_and_its_tag() {
    let sch = schema(EVENTS);
    let u = sch.union("Event").expect("Event should be a union");

    assert_eq!(u.tag, "type");
    assert_eq!(u.name, "Event");
    let tags: Vec<&str> = u.variants.iter().map(|v| v.tag.as_str()).collect();
    assert_eq!(tags, vec!["created", "deleted"]);
    assert_eq!(u.variants[0].type_name, "Created");

    // A union is not an object type, and both share one namespace.
    assert!(sch.get("Event").is_none());
    assert!(sch.declares("Event"));
}

#[test]
fn the_tag_is_required_because_nothing_is_guessed() {
    // No default of `type`, no inference from the variants. A union that
    // guessed which field decides would be guessing what the data means.
    let message = why("schema A { x: u8 }\nunion U { a: A }\n");
    assert!(message.contains("@tag"), "unhelpful message: {message}");
}

#[test]
fn a_variant_must_name_a_declared_schema() {
    assert!(why("union U @tag(\"t\") { a: Nope }\n").contains("unknown type `Nope`"));
    assert!(why("union U @tag(\"t\") { a: String }\n").contains("built-in"));
}

#[test]
fn a_variant_may_not_be_another_union() {
    // Resolving it would need a second discriminant, and nothing in the
    // payload says which one to read first.
    let src = "schema A { x: u8 }\nunion Inner @tag(\"t\") { a: A }\nunion Outer @tag(\"u\") { i: Inner }\n";
    assert!(why(src).contains("is a union"));
}

#[test]
fn a_variant_may_not_declare_the_tag_itself() {
    // Two sources of truth for one value, which could then disagree.
    let src = "schema A { type: String\n x: u8 }\nunion U @tag(\"type\") { a: A }\n";
    let message = why(src);
    assert!(message.contains("tag"), "unhelpful message: {message}");
}

#[test]
fn a_union_and_a_schema_cannot_share_a_name() {
    let src = "schema A { x: u8 }\nschema Event { y: u8 }\nunion Event @tag(\"t\") { a: A }\n";
    assert!(why(src).contains("declared more than once"));

    let src = "schema A { x: u8 }\nunion Event @tag(\"t\") { a: A }\nschema Event { y: u8 }\n";
    assert!(why(src).contains("declared more than once"));
}

#[test]
fn a_union_needs_a_variant_and_may_not_repeat_a_tag() {
    assert!(why("union U @tag(\"t\") { }\n").contains("at least one variant"));

    let src = "schema A { x: u8 }\nschema B { y: u8 }\nunion U @tag(\"t\") { a: A\n a: B }\n";
    assert!(why(src).contains("twice"));
}

#[test]
fn a_variant_may_be_declared_after_the_union_that_uses_it() {
    // The same second pass that lets a field reference a later schema.
    let src = "union U @tag(\"t\") { a: A }\nschema A { x: u8 }\n";
    let sch = schema(src);
    assert_eq!(
        codes(
            &sch,
            "U",
            &obj(&[("t", s("a")), ("x", Value::Int(Int::from(1_u64)))])
        ),
        vec![]
    );
}

#[test]
fn a_tag_value_may_be_a_quoted_string() {
    // Wire tags are not always identifiers: `user.created` is common.
    let src = "schema A { x: u8 }\nunion U @tag(\"type\") { \"user.created\": A }\n";
    let sch = schema(src);
    let ok = obj(&[
        ("type", s("user.created")),
        ("x", Value::Int(Int::from(1_u64))),
    ]);
    assert_eq!(codes(&sch, "U", &ok), vec![]);
}

// --- validating -------------------------------------------------------------

fn created() -> Value {
    obj(&[
        ("type", s("created")),
        ("who", s("Gabriel")),
        ("amount", Value::Int(Int::from(5_u64))),
    ])
}

#[test]
fn the_tag_selects_the_variant_and_the_variant_is_checked() {
    let sch = schema(EVENTS);
    assert_eq!(codes(&sch, "Event", &created()), vec![]);

    // `deleted` has no `amount`, so the same payload under the other tag is
    // both missing a field and carrying an unknown one.
    let other = obj(&[
        ("type", s("deleted")),
        ("who", s("Gabriel")),
        ("amount", Value::Int(Int::from(5_u64))),
    ]);
    assert_eq!(
        codes(&sch, "Event", &other),
        vec![("amount".into(), Code::UnknownField)]
    );
}

#[test]
fn the_tag_is_not_an_unknown_field_of_the_variant() {
    // `Created` does not declare `type`; the union accounted for it. This is
    // the whole reason a variant may not declare the tag itself.
    let sch = schema(EVENTS);
    assert_eq!(codes(&sch, "Event", &created()), vec![]);
}

#[test]
fn a_missing_tag_is_required_at_the_tag_s_own_path() {
    let sch = schema(EVENTS);
    let v = obj(&[
        ("who", s("Gabriel")),
        ("amount", Value::Int(Int::from(5_u64))),
    ]);
    assert_eq!(
        codes(&sch, "Event", &v),
        vec![("type".into(), Code::Required)]
    );
}

#[test]
fn a_null_tag_is_null_not_allowed() {
    let sch = schema(EVENTS);
    let v = obj(&[("type", Value::Null), ("who", s("G"))]);
    assert_eq!(
        codes(&sch, "Event", &v),
        vec![("type".into(), Code::NullNotAllowed)]
    );
}

#[test]
fn a_tag_that_is_not_a_string_is_a_type_mismatch() {
    let sch = schema(EVENTS);
    let v = obj(&[("type", Value::Int(Int::from(1_u64))), ("who", s("G"))]);
    assert_eq!(
        codes(&sch, "Event", &v),
        vec![("type".into(), Code::TypeMismatch)]
    );
}

#[test]
fn an_unlisted_tag_is_unknown_variant_and_stops_there() {
    // Nothing else is reported: with no variant chosen there is no shape to
    // check the rest against, and inventing one would produce a list of
    // errors about a payload the caller never claimed to send.
    let sch = schema(EVENTS);
    let v = obj(&[("type", s("archived")), ("nonsense", s("x"))]);
    assert_eq!(
        codes(&sch, "Event", &v),
        vec![("type".into(), Code::UnknownVariant)]
    );
}

#[test]
fn the_unknown_variant_message_lists_what_was_expected() {
    let sch = schema(EVENTS);
    let v = obj(&[("type", s("archived"))]);
    let err = validate(&sch, "Event", &v, Limits::DEFAULT).unwrap_err();
    assert!(
        err.issues[0].message.contains("created, deleted"),
        "unhelpful message: {}",
        err.issues[0].message
    );
}

#[test]
fn a_union_is_not_an_object() {
    let sch = schema(EVENTS);
    assert_eq!(
        codes(&sch, "Event", &s("created")),
        vec![("<root>".into(), Code::TypeMismatch)]
    );
}

// --- unions in the places types go ------------------------------------------

const ENVELOPE: &str = r#"
schema Created {
  who:    String
  amount: u64
}

schema Deleted {
  who:    String
}

union Event @tag("type") {
  created: Created
  deleted: Deleted
}

schema Envelope {
  id:     u64
  latest: Event
  log:    [Event]
  draft:  optional Event?
}
"#;

fn envelope(latest: Value, log: Vec<Value>) -> Value {
    obj(&[
        ("id", Value::Int(Int::from(1_u64))),
        ("latest", latest),
        ("log", Value::Array(log)),
    ])
}

#[test]
fn a_union_may_be_the_type_of_a_field() {
    let sch = schema(ENVELOPE);
    assert_eq!(
        codes(&sch, "Envelope", &envelope(created(), vec![])),
        vec![]
    );
}

#[test]
fn an_issue_inside_a_union_valued_field_carries_the_field_s_path() {
    // A variant is not a level of nesting: it is `latest.type`, never
    // `latest.created.type`.
    let sch = schema(ENVELOPE);
    let bad = obj(&[("type", s("nope"))]);
    assert_eq!(
        codes(&sch, "Envelope", &envelope(bad, vec![])),
        vec![("latest.type".into(), Code::UnknownVariant)]
    );
}

#[test]
fn a_union_inside_an_array_reports_the_element_s_own_index() {
    let sch = schema(ENVELOPE);
    let bad = obj(&[("type", s("created")), ("who", s("G"))]);
    assert_eq!(
        codes(&sch, "Envelope", &envelope(created(), vec![created(), bad])),
        vec![("log[1].amount".into(), Code::Required)]
    );
}

#[test]
fn presence_applies_to_a_union_valued_field_like_any_other() {
    let sch = schema(ENVELOPE);

    // `draft` is optional and nullable, so both absent and null are fine.
    assert_eq!(
        codes(&sch, "Envelope", &envelope(created(), vec![])),
        vec![]
    );

    let mut pairs = vec![
        ("id", Value::Int(Int::from(1_u64))),
        ("latest", created()),
        ("log", Value::Array(vec![])),
        ("draft", Value::Null),
    ];
    assert_eq!(codes(&sch, "Envelope", &obj(&pairs)), vec![]);

    // `latest` is neither, so a null there is refused.
    pairs[1] = ("latest", Value::Null);
    assert_eq!(
        codes(&sch, "Envelope", &obj(&pairs)),
        vec![("latest".into(), Code::NullNotAllowed)]
    );
}

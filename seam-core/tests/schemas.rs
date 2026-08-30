//! End to end over the real `.seam` files in the repository: parse the file on
//! disk, then validate payloads against what came out.
//!
//! Integration tests build as their own crate, so the workspace `panic`/`expect`
//! denials do not see the `cfg(test)` allow in `lib.rs`. Relaxed here only.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use seam_core::{
    error::Code,
    parse,
    schema::{Presence, Schema},
    validate,
    value::{Int, Value},
    Limits,
};

fn load(name: &str) -> Schema {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../conformance/schemas")
        .join(format!("{name}.seam"));
    let src = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    parse(&src).unwrap_or_else(|e| panic!("parsing {}: {e}", path.display()))
}

fn obj(pairs: &[(&str, Value)]) -> Value {
    let mut m = BTreeMap::new();
    for (k, v) in pairs {
        m.insert((*k).to_string(), v.clone());
    }
    Value::Object(m)
}

fn base() -> Vec<(&'static str, Value)> {
    vec![
        ("id", Value::Int(Int::from(1_u64))),
        ("name", Value::String("Gabriel".into())),
        ("plan", Value::String("pro".into())),
        ("nickname", Value::Null),
    ]
}

fn with(key: &'static str, value: Value) -> Value {
    let mut pairs = base();
    pairs.push((key, value));
    obj(&pairs)
}

fn codes(v: &Value) -> Vec<(String, Code)> {
    match validate(&load("user"), "User", v, Limits::DEFAULT) {
        Ok(()) => vec![],
        Err(e) => e
            .issues
            .into_iter()
            .map(|i| (i.path.render(), i.code))
            .collect(),
    }
}

#[test]
fn the_conformance_schema_parses() {
    let schema = load("user");
    let user = schema.get("User").expect("User should be declared");

    assert_eq!(user.fields.len(), 9);
    assert!(user.deny_unknown_fields);

    let presence = |n: &str| user.field(n).map(|f| f.presence);
    assert_eq!(presence("id"), Some(Presence::required()));
    assert_eq!(presence("nickname"), Some(Presence::nullable()));
    assert_eq!(presence("bio"), Some(Presence::optional()));
    assert_eq!(presence("avatar"), Some(Presence::optional_nullable()));
}

#[test]
fn a_valid_payload_passes() {
    assert_eq!(codes(&obj(&base())), vec![]);
}

#[test]
fn the_readme_integer_survives_the_boundary() {
    let v = with("id", Value::Int(Int::from(9_007_199_254_740_993_u64)));
    assert_eq!(codes(&v), vec![]);
}

#[test]
fn a_naive_datetime_is_rejected() {
    let v = with("last_seen", Value::String("2026-08-29T14:30:00".into()));
    assert_eq!(codes(&v), vec![("last_seen".into(), Code::MissingTimezone)]);

    let ok = with(
        "last_seen",
        Value::String("2026-08-29T14:30:00-05:00".into()),
    );
    assert_eq!(codes(&ok), vec![]);
}

#[test]
fn a_date_does_not_accept_an_instant() {
    let v = with("signup_date", Value::String("2026-08-29T00:00:00Z".into()));
    assert_eq!(codes(&v), vec![("signup_date".into(), Code::InvalidDate)]);
}

#[test]
fn absence_and_null_stay_distinct_through_the_file() {
    // `bio` is optional but not nullable.
    assert_eq!(
        codes(&with("bio", Value::Null)),
        vec![("bio".into(), Code::NullNotAllowed)]
    );
    // `avatar` is both, so null is fine.
    assert_eq!(codes(&with("avatar", Value::Null)), vec![]);
}

#[test]
fn rules_from_the_file_are_enforced() {
    let mut pairs = base();
    pairs.retain(|(k, _)| *k != "name");
    pairs.push(("name", Value::String("ab".into())));
    assert_eq!(codes(&obj(&pairs)), vec![("name".into(), Code::TooShort)]);

    let many = Value::Array(vec![Value::String("t".into()); 11]);
    assert_eq!(
        codes(&with("tags", many)),
        vec![("tags".into(), Code::TooManyItems)]
    );
}

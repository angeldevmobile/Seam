//! One walk over the payload, reading it in place.
//!
//! Generic over [`Input`] so a binding validates its host's own objects without
//! copying them into a [`Value`] first. Everything here reads; nothing here
//! allocates a representation of the payload.

use crate::datetime::{validate_date, validate_datetime};
use crate::error::{Code, Issue, Path, Segment, ValidationError};
use crate::input::{Input, Kind};
use crate::limits::Limits;
use crate::schema::{Field, IntType, ObjectType, Presence, Rule, Schema, Type};
use crate::value::{Int, Slot, Value};

pub fn validate<I: Input>(
    schema: &Schema,
    type_name: &str,
    input: &I,
    limits: Limits,
) -> Result<(), ValidationError> {
    let mut v = Validator { schema, limits, path: Vec::new(), issues: Vec::new() };

    match schema.get(type_name) {
        Some(ty) => v.object(ty, input, 0),
        None => v.push(
            Code::UnknownType,
            format!("schema declares no type named `{type_name}`"),
        ),
    }

    if v.issues.is_empty() {
        Ok(())
    } else {
        Err(ValidationError { issues: v.issues })
    }
}

enum Step<'a> {
    Key(&'a str),
    Index(usize),
}

struct Validator<'a> {
    schema: &'a Schema,
    limits: Limits,
    path: Vec<Step<'a>>,
    issues: Vec<Issue>,
}

impl<'a> Validator<'a> {
    fn here(&self) -> Path {
        Path(
            self.path
                .iter()
                .map(|step| match step {
                    Step::Key(k) => Segment::Key((*k).to_string()),
                    Step::Index(i) => Segment::Index(*i),
                })
                .collect(),
        )
    }

    fn push(&mut self, code: Code, message: String) {
        let path = self.here();
        self.issues.push(Issue { path, code, message });
    }

    fn push_under(&mut self, key: String, code: Code, message: String) {
        let mut path = self.here();
        path.0.push(Segment::Key(key));
        self.issues.push(Issue { path, code, message });
    }

    fn within<F: FnOnce(&mut Self)>(&mut self, step: Step<'a>, f: F) {
        self.path.push(step);
        f(self);
        self.path.pop();
    }

    fn depth_ok(&mut self, depth: usize) -> bool {
        if depth > self.limits.max_depth {
            self.push(
                Code::DepthExceeded,
                format!("nesting deeper than the limit of {}", self.limits.max_depth),
            );
            return false;
        }
        true
    }

    fn mismatch<I: Input>(&mut self, expected: &str, got: &I) {
        self.push(
            Code::TypeMismatch,
            format!("expected {expected}, found {}", got.kind().name()),
        );
    }

    fn object<I: Input>(&mut self, ty: &'a ObjectType, input: &I, depth: usize) {
        if !self.depth_ok(depth) {
            return;
        }
        if input.kind() != Kind::Object {
            self.mismatch("object", input);
            return;
        }

        if input.len() > self.limits.max_object_keys {
            self.push(
                Code::SizeExceeded,
                format!(
                    "{} keys exceeds the limit of {}",
                    input.len(),
                    self.limits.max_object_keys
                ),
            );
            return;
        }

        for field in &ty.fields {
            self.field(field, input, depth);
        }

        if ty.deny_unknown_fields {
            // Collected first because the callback cannot borrow `self` while
            // it is already borrowed to walk the input.
            let mut unknown = Vec::new();
            input.each_key(&mut |key| {
                if ty.field(key).is_none() {
                    unknown.push(key.to_string());
                }
            });
            let owner = ty.name.clone();
            for key in unknown {
                let message = format!("`{owner}` declares no field named `{key}`");
                self.push_under(key, Code::UnknownField, message);
            }
        }
    }

    fn field<I: Input>(&mut self, field: &'a Field, input: &I, depth: usize) {
        let slot = input.slot(&field.name);
        self.within(Step::Key(&field.name), |v| match slot {
            Slot::Absent => {
                if !field.presence.optional {
                    v.push(Code::Required, required_message(field.presence));
                }
            }
            Slot::Null => {
                if !field.presence.nullable {
                    v.push(Code::NullNotAllowed, null_message(field.presence));
                }
            }
            Slot::Present(value) => {
                v.value(&field.ty, &value, depth + 1);
                v.rules(&field.rules, &value);
            }
        });
    }

    fn value<I: Input>(&mut self, ty: &'a Type, input: &I, depth: usize) {
        // Applies to every string-shaped type, so it is checked once here
        // rather than in each of String, Date, DateTime and Enum.
        if input.kind() == Kind::String {
            if let Some(s) = input.as_str() {
                if s.len() > self.limits.max_string_bytes {
                    self.push(
                        Code::SizeExceeded,
                        format!(
                            "{} bytes exceeds the limit of {}",
                            s.len(),
                            self.limits.max_string_bytes
                        ),
                    );
                    return;
                }
            }
        }

        match ty {
            Type::Bool => {
                if input.as_bool().is_none() {
                    self.mismatch("bool", input);
                }
            }
            Type::Float => match input.kind() {
                Kind::Int => {}
                Kind::Float => match input.as_f64() {
                    Some(f) if f.is_finite() => {}
                    _ => self.push(
                        Code::NotFinite,
                        "NaN and infinity are not valid values".to_string(),
                    ),
                },
                _ => self.mismatch("float", input),
            },
            Type::String => {
                if input.kind() != Kind::String {
                    self.mismatch("string", input);
                }
            }
            Type::Int(int_ty) => self.integer(*int_ty, input),
            Type::Date => self.temporal(input, "date", validate_date),
            Type::DateTime => self.temporal(input, "datetime", validate_datetime),
            Type::Enum(allowed) => self.enumeration(allowed, input),
            Type::Array { item, item_nullable } => self.array(item, *item_nullable, input, depth),
            Type::Object(obj) => self.object(obj, input, depth),
            Type::Ref(name) => {
                // Bound first so the borrow of the schema is not tied to the
                // mutable borrow of `self` that follows.
                let target = self.schema.get(name);
                match target {
                    Some(obj) => self.object(obj, input, depth),
                    None => self.push(
                        Code::UnknownType,
                        format!("schema declares no type named `{name}`"),
                    ),
                }
            }
        }
    }

    fn integer<I: Input>(&mut self, ty: IntType, input: &I) {
        if input.kind() == Kind::UnsafeInteger {
            self.push(
                Code::UnsafeInteger,
                "arrived in a numeric type that cannot hold it exactly;                  send it as the host's arbitrary-precision integer"
                    .to_string(),
            );
            return;
        }
        if input.kind() == Kind::IntegerTooWide {
            self.push(
                Code::IntegerTooWide,
                "integer is wider than 64 bits".to_string(),
            );
            return;
        }
        let Some(n) = input.as_int() else {
            self.mismatch(ty.name(), input);
            return;
        };
        let (min, max) = ty.range();
        let v = n.as_i128();
        if v < min || v > max {
            self.push(
                Code::OutOfRange,
                format!("{v} is outside the range of {} ({min}..={max})", ty.name()),
            );
        }
    }

    fn temporal<I: Input>(
        &mut self,
        input: &I,
        expected: &str,
        check: fn(&str) -> Result<(), Code>,
    ) {
        let Some(s) = input.as_str() else {
            self.mismatch(expected, input);
            return;
        };
        if let Err(code) = check(&s) {
            let message = match code {
                Code::MissingTimezone => {
                    format!("`{s}` has no UTC offset; Seam does not assume local time")
                }
                _ => format!("`{s}` is not a valid {expected}"),
            };
            self.push(code, message);
        }
    }

    fn enumeration<I: Input>(&mut self, allowed: &[String], input: &I) {
        let Some(s) = input.as_str() else {
            self.mismatch("string", input);
            return;
        };
        if !allowed.iter().any(|a| a.as_str() == s.as_ref()) {
            self.push(
                Code::NotInEnum,
                format!("`{s}` is not one of: {}", allowed.join(", ")),
            );
        }
    }

    fn array<I: Input>(&mut self, item: &'a Type, item_nullable: bool, input: &I, depth: usize) {
        if !self.depth_ok(depth) {
            return;
        }
        if input.kind() != Kind::Array {
            self.mismatch("array", input);
            return;
        }
        let len = input.len();
        if len > self.limits.max_items {
            self.push(
                Code::SizeExceeded,
                format!("{len} items exceeds the limit of {}", self.limits.max_items),
            );
            return;
        }
        for i in 0..len {
            let Some(child) = input.item(i) else { continue };
            self.within(Step::Index(i), |v| match child.kind() {
                Kind::Null if item_nullable => {}
                Kind::Null => v.push(
                    Code::NullNotAllowed,
                    "must not be null; declare the element as `T?` to allow it".to_string(),
                ),
                _ => v.value(item, &child, depth + 1),
            });
        }
    }

    fn rules<I: Input>(&mut self, rules: &[Rule], input: &I) {
        // Counted once: `as_str` may decode, and asking per rule would pay for
        // it twice on a field carrying both bounds.
        let chars = rules
            .iter()
            .any(|r| matches!(r, Rule::MinLen(_) | Rule::MaxLen(_)))
            .then(|| input.as_str().map(|s| s.chars().count()))
            .flatten();

        for rule in rules {
            match rule {
                Rule::MinLen(n) => {
                    if let Some(len) = chars {
                        if len < *n {
                            self.push(
                                Code::TooShort,
                                format!("length {len} is below the minimum of {n}"),
                            );
                        }
                    }
                }
                Rule::MaxLen(n) => {
                    if let Some(len) = chars {
                        if len > *n {
                            self.push(
                                Code::TooLong,
                                format!("length {len} exceeds the maximum of {n}"),
                            );
                        }
                    }
                }
                Rule::Range { min, max } => {
                    if let Some(n) = input.as_int() {
                        let v = n.as_i128();
                        if v < *min || v > *max {
                            self.push(Code::OutOfRange, format!("{v} is outside {min}..={max}"));
                        }
                    }
                }
                Rule::MinItems(n) => {
                    if input.kind() == Kind::Array && input.len() < *n {
                        self.push(
                            Code::TooFewItems,
                            format!("{} items is below the minimum of {n}", input.len()),
                        );
                    }
                }
                Rule::MaxItems(n) => {
                    if input.kind() == Kind::Array && input.len() > *n {
                        self.push(
                            Code::TooManyItems,
                            format!("{} items exceeds the maximum of {n}", input.len()),
                        );
                    }
                }
            }
        }
    }
}

fn required_message(p: Presence) -> String {
    if p.nullable {
        "required; may be null but must be present".to_string()
    } else {
        "required".to_string()
    }
}

fn null_message(p: Presence) -> String {
    if p.optional {
        "may be absent, but must not be null when present".to_string()
    } else {
        "must not be null".to_string()
    }
}

impl From<Int> for Value {
    fn from(i: Int) -> Self {
        Value::Int(i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::IntWidth;
    use std::collections::BTreeMap;

    fn user_schema() -> Schema {
        let user = ObjectType {
            name: "User".into(),
            deny_unknown_fields: true,
            fields: vec![
                Field {
                    name: "id".into(),
                    ty: Type::Int(IntType { width: IntWidth::W64, signed: false }),
                    presence: Presence::required(),
                    rules: vec![],
                },
                Field {
                    name: "name".into(),
                    ty: Type::String,
                    presence: Presence::required(),
                    rules: vec![Rule::MinLen(3), Rule::MaxLen(64)],
                },
                Field {
                    name: "plan".into(),
                    ty: Type::Enum(vec!["free".into(), "pro".into(), "enterprise".into()]),
                    presence: Presence::required(),
                    rules: vec![],
                },
                Field {
                    name: "nickname".into(),
                    ty: Type::String,
                    presence: Presence::nullable(),
                    rules: vec![],
                },
                Field {
                    name: "bio".into(),
                    ty: Type::String,
                    presence: Presence::optional(),
                    rules: vec![],
                },
                Field {
                    name: "signup_date".into(),
                    ty: Type::Date,
                    presence: Presence::optional_nullable(),
                    rules: vec![],
                },
                Field {
                    name: "tags".into(),
                    ty: Type::Array { item: Box::new(Type::String), item_nullable: false },
                    presence: Presence::optional(),
                    rules: vec![Rule::MaxItems(3)],
                },
            ],
        };
        let mut schema = Schema::default();
        schema.types.insert("User".into(), user);
        schema
    }

    fn obj(pairs: &[(&str, Value)]) -> Value {
        let mut m = BTreeMap::new();
        for (k, v) in pairs {
            m.insert((*k).to_string(), v.clone());
        }
        Value::Object(m)
    }

    fn valid_user() -> Value {
        obj(&[
            ("id", Value::Int(Int::from(1_u64))),
            ("name", Value::String("Gabriel".into())),
            ("plan", Value::String("pro".into())),
            ("nickname", Value::Null),
        ])
    }

    fn with(key: &str, value: Value) -> Value {
        let mut v = valid_user();
        if let Value::Object(m) = &mut v {
            m.insert(key.into(), value);
        }
        v
    }

    fn codes(v: &Value) -> Vec<(String, Code)> {
        match validate(&user_schema(), "User", v, Limits::DEFAULT) {
            Ok(()) => vec![],
            Err(e) => e
                .issues
                .into_iter()
                .map(|i| (i.path.render(), i.code))
                .collect(),
        }
    }

    #[test]
    fn a_valid_payload_passes() {
        assert_eq!(codes(&valid_user()), vec![]);
    }

    #[test]
    fn the_readme_payload_keeps_its_u64_exactly() {
        let v = with("id", Value::Int(Int::from(9_007_199_254_740_993_u64)));
        assert_eq!(codes(&v), vec![]);
    }

    #[test]
    fn absent_and_null_are_judged_separately() {
        // nullable but required: null is fine, absent is not.
        let mut m = BTreeMap::new();
        m.insert("id".into(), Value::Int(Int::from(1_u64)));
        m.insert("name".into(), Value::String("Gabriel".into()));
        m.insert("plan".into(), Value::String("pro".into()));
        assert_eq!(
            codes(&Value::Object(m)),
            vec![("nickname".into(), Code::Required)]
        );

        // optional but not nullable: absent is fine, null is not.
        assert_eq!(
            codes(&with("bio", Value::Null)),
            vec![("bio".into(), Code::NullNotAllowed)]
        );

        // optional and nullable: both are fine.
        assert_eq!(codes(&with("signup_date", Value::Null)), vec![]);
    }

    #[test]
    fn a_naive_datetime_never_slips_through() {
        let v = with("signup_date", Value::String("2026-08-29T00:00:00".into()));
        assert_eq!(codes(&v), vec![("signup_date".into(), Code::InvalidDate)]);
    }

    #[test]
    fn errors_carry_the_path_into_arrays() {
        let v = with(
            "tags",
            Value::Array(vec![
                Value::String("ok".into()),
                Value::Int(Int::from(7_i64)),
            ]),
        );
        assert_eq!(codes(&v), vec![("tags[1]".into(), Code::TypeMismatch)]);
    }

    #[test]
    fn every_issue_is_reported_not_just_the_first() {
        let v = obj(&[
            ("id", Value::String("not a number".into())),
            ("name", Value::String("ab".into())),
            ("plan", Value::String("platinum".into())),
            ("nickname", Value::Null),
            ("surprise", Value::Bool(true)),
        ]);
        let found = codes(&v);
        assert_eq!(found.len(), 4, "expected four issues, got {found:?}");
        assert!(found.contains(&("id".into(), Code::TypeMismatch)));
        assert!(found.contains(&("name".into(), Code::TooShort)));
        assert!(found.contains(&("plan".into(), Code::NotInEnum)));
        assert!(found.contains(&("surprise".into(), Code::UnknownField)));
    }

    #[test]
    fn integer_width_is_enforced_against_the_declared_type() {
        let mut schema = Schema::default();
        schema.types.insert(
            "T".into(),
            ObjectType {
                name: "T".into(),
                deny_unknown_fields: false,
                fields: vec![Field {
                    name: "n".into(),
                    ty: Type::Int(IntType { width: IntWidth::W32, signed: true }),
                    presence: Presence::required(),
                    rules: vec![],
                }],
            },
        );
        let too_big = obj(&[("n", Value::Int(Int::from(i64::from(i32::MAX) + 1)))]);
        let err = validate(&schema, "T", &too_big, Limits::DEFAULT);
        assert!(matches!(err, Err(ref e) if e.issues.len() == 1));
        if let Err(e) = err {
            assert_eq!(e.issues.first().map(|i| i.code), Some(Code::OutOfRange));
        }
    }

    #[test]
    fn limits_stop_an_oversized_array() {
        let limits = Limits { max_items: 2, ..Limits::DEFAULT };
        let v = with("tags", Value::Array(vec![Value::String("a".into()); 5]));
        let err = validate(&user_schema(), "User", &v, limits);
        assert!(matches!(err, Err(ref e) if e.issues.iter().any(|i| i.code == Code::SizeExceeded)));
    }

    #[test]
    fn limits_stop_an_oversized_string() {
        let limits = Limits { max_string_bytes: 8, ..Limits::DEFAULT };
        let v = with("name", Value::String("a".repeat(9)));
        let err = validate(&user_schema(), "User", &v, limits);
        assert!(matches!(err, Err(ref e) if e.issues.iter().any(|i| i.code == Code::SizeExceeded)));

        let ok = with("name", Value::String("a".repeat(8)));
        assert!(validate(&user_schema(), "User", &ok, limits).is_ok());
    }

    /// The limit is in bytes, so a multi-byte character counts as its encoded
    /// width. A limit measured in characters would not bound memory.
    #[test]
    fn the_string_limit_counts_bytes_not_characters() {
        let limits = Limits { max_string_bytes: 4, ..Limits::DEFAULT };
        // Three characters, six bytes in UTF-8.
        let v = with("name", Value::String("ñññ".into()));
        let err = validate(&user_schema(), "User", &v, limits);
        assert!(matches!(err, Err(ref e) if e.issues.iter().any(|i| i.code == Code::SizeExceeded)));
    }

    #[test]
    fn a_null_array_item_is_rejected_unless_the_element_allows_it() {
        let with_items = |item_nullable| {
            let mut schema = Schema::default();
            schema.types.insert(
                "T".into(),
                ObjectType {
                    name: "T".into(),
                    deny_unknown_fields: false,
                    fields: vec![Field {
                        name: "xs".into(),
                        ty: Type::Array { item: Box::new(Type::String), item_nullable },
                        presence: Presence::required(),
                        rules: vec![],
                    }],
                },
            );
            schema
        };
        let payload = obj(&[(
            "xs",
            Value::Array(vec![Value::String("a".into()), Value::Null]),
        )]);

        let err = validate(&with_items(false), "T", &payload, Limits::DEFAULT)
            .expect_err("a null element must not pass `[String]`");
        assert_eq!(err.issues.len(), 1);
        assert_eq!(
            err.issues.first().map(|i| (i.path.render(), i.code)),
            Some(("xs[1]".to_string(), Code::NullNotAllowed))
        );

        assert!(validate(&with_items(true), "T", &payload, Limits::DEFAULT).is_ok());
    }
}

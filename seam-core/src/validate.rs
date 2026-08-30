use std::collections::BTreeMap;

use crate::datetime::{validate_date, validate_datetime};
use crate::error::{Code, Issue, Path, Segment, ValidationError};
use crate::limits::Limits;
use crate::schema::{Field, IntType, ObjectType, Presence, Rule, Schema, Type};
use crate::value::{Int, Slot, Value};

pub fn validate(
    schema: &Schema,
    type_name: &str,
    value: &Value,
    limits: Limits,
) -> Result<(), ValidationError> {
    let mut v = Validator { schema, limits, path: Vec::new(), issues: Vec::new() };

    match schema.get(type_name) {
        Some(ty) => v.object(ty, value, 0),
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

struct Validator<'a> {
    schema: &'a Schema,
    limits: Limits,
    path: Vec<Segment>,
    issues: Vec<Issue>,
}

impl Validator<'_> {
    fn push(&mut self, code: Code, message: String) {
        self.issues
            .push(Issue { path: Path(self.path.clone()), code, message });
    }

    fn within<F: FnOnce(&mut Self)>(&mut self, seg: Segment, f: F) {
        self.path.push(seg);
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

    fn object(&mut self, ty: &ObjectType, value: &Value, depth: usize) {
        if !self.depth_ok(depth) {
            return;
        }
        let Value::Object(map) = value else {
            self.push(
                Code::TypeMismatch,
                format!("expected object, found {}", value.kind()),
            );
            return;
        };

        if map.len() > self.limits.max_object_keys {
            self.push(
                Code::SizeExceeded,
                format!(
                    "{} keys exceeds the limit of {}",
                    map.len(),
                    self.limits.max_object_keys
                ),
            );
            return;
        }

        for field in &ty.fields {
            self.field(field, map, depth);
        }

        if ty.deny_unknown_fields {
            let owner = ty.name.clone();
            for key in map.keys() {
                if ty.field(key).is_none() {
                    let message = format!("`{owner}` declares no field named `{key}`");
                    self.within(Segment::Key(key.clone()), |v| {
                        v.push(Code::UnknownField, message);
                    });
                }
            }
        }
    }

    fn field(&mut self, field: &Field, map: &BTreeMap<String, Value>, depth: usize) {
        let slot = Slot::read(map, &field.name);
        self.within(Segment::Key(field.name.clone()), |v| match slot {
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
                v.value(&field.ty, value, depth + 1);
                v.rules(&field.rules, value);
            }
        });
    }

    fn value(&mut self, ty: &Type, value: &Value, depth: usize) {
        // Applies to every string-shaped type, so it is checked once here
        // rather than in each of String, Date, DateTime and Enum.
        if let Value::String(s) = value {
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

        match ty {
            Type::Bool => self.expect(matches!(value, Value::Bool(_)), "bool", value),
            Type::Float => match value {
                Value::Float(f) if f.is_finite() => {}
                Value::Float(_) => self.push(
                    Code::NotFinite,
                    "NaN and infinity are not valid values".to_string(),
                ),
                // An integer is an acceptable float; the reverse is not.
                Value::Int(_) => {}
                other => self.expect(false, "float", other),
            },
            Type::String => self.expect(matches!(value, Value::String(_)), "string", value),
            Type::Int(int_ty) => self.integer(*int_ty, value),
            Type::Date => self.temporal(value, "date", validate_date),
            Type::DateTime => self.temporal(value, "datetime", validate_datetime),
            Type::Enum(allowed) => self.enumeration(allowed, value),
            Type::Array { item, item_nullable } => {
                self.array(item, *item_nullable, value, depth);
            }
            Type::Object(obj) => self.object(obj, value, depth),
            Type::Ref(name) => match self.schema.get(name) {
                Some(obj) => self.object(obj, value, depth),
                None => self.push(
                    Code::UnknownType,
                    format!("schema declares no type named `{name}`"),
                ),
            },
        }
    }

    fn expect(&mut self, ok: bool, expected: &str, value: &Value) {
        if !ok {
            self.push(
                Code::TypeMismatch,
                format!("expected {expected}, found {}", value.kind()),
            );
        }
    }

    fn integer(&mut self, ty: IntType, value: &Value) {
        let Value::Int(n) = value else {
            self.expect(false, ty.name(), value);
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

    fn temporal(&mut self, value: &Value, expected: &str, check: fn(&str) -> Result<(), Code>) {
        let Value::String(s) = value else {
            self.expect(false, expected, value);
            return;
        };
        if let Err(code) = check(s) {
            let message = match code {
                Code::MissingTimezone => {
                    format!("`{s}` has no UTC offset; Seam does not assume local time")
                }
                _ => format!("`{s}` is not a valid {expected}"),
            };
            self.push(code, message);
        }
    }

    fn enumeration(&mut self, allowed: &[String], value: &Value) {
        let Value::String(s) = value else {
            self.expect(false, "string", value);
            return;
        };
        if !allowed.iter().any(|a| a == s) {
            self.push(
                Code::NotInEnum,
                format!("`{s}` is not one of: {}", allowed.join(", ")),
            );
        }
    }

    fn array(&mut self, item: &Type, item_nullable: bool, value: &Value, depth: usize) {
        if !self.depth_ok(depth) {
            return;
        }
        let Value::Array(items) = value else {
            self.expect(false, "array", value);
            return;
        };
        if items.len() > self.limits.max_items {
            self.push(
                Code::SizeExceeded,
                format!(
                    "{} items exceeds the limit of {}",
                    items.len(),
                    self.limits.max_items
                ),
            );
            return;
        }
        for (i, v) in items.iter().enumerate() {
            self.within(Segment::Index(i), |validator| match v {
                Value::Null if item_nullable => {}
                Value::Null => validator.push(
                    Code::NullNotAllowed,
                    "must not be null; declare the element as `T?` to allow it".to_string(),
                ),
                v => validator.value(item, v, depth + 1),
            });
        }
    }

    fn rules(&mut self, rules: &[Rule], value: &Value) {
        for rule in rules {
            match (rule, value) {
                (Rule::MinLen(n), Value::String(s)) => {
                    let len = s.chars().count();
                    if len < *n {
                        self.push(
                            Code::TooShort,
                            format!("length {len} is below the minimum of {n}"),
                        );
                    }
                }
                (Rule::MaxLen(n), Value::String(s)) => {
                    let len = s.chars().count();
                    if len > *n {
                        self.push(
                            Code::TooLong,
                            format!("length {len} exceeds the maximum of {n}"),
                        );
                    }
                }
                (Rule::Range { min, max }, Value::Int(n)) => {
                    let v = n.as_i128();
                    if v < *min || v > *max {
                        self.push(Code::OutOfRange, format!("{v} is outside {min}..={max}"));
                    }
                }
                (Rule::MinItems(n), Value::Array(items)) => {
                    if items.len() < *n {
                        self.push(
                            Code::TooFewItems,
                            format!("{} items is below the minimum of {n}", items.len()),
                        );
                    }
                }
                (Rule::MaxItems(n), Value::Array(items)) => {
                    if items.len() > *n {
                        self.push(
                            Code::TooManyItems,
                            format!("{} items exceeds the maximum of {n}", items.len()),
                        );
                    }
                }
                // A rule that does not apply to this kind is silent: the type
                // check already reported the mismatch.
                _ => {}
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

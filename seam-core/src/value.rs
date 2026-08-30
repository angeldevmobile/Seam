use std::collections::BTreeMap;

use crate::input::Kind;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(Int),
    Float(f64),
    String(String),
    Array(Vec<Value>),
    Object(BTreeMap<String, Value>),
    /// An integer too wide for `Int`, kept as a fact so validation can report
    /// it at its own path instead of the parse failing the whole document.
    IntTooWide,
}

/// An integer that has not been through a float. Keeping the source signedness
/// is what lets a `u64` above `i64::MAX` survive the boundary intact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Int {
    Signed(i64),
    Unsigned(u64),
}

impl Int {
    pub fn as_i128(self) -> i128 {
        match self {
            Int::Signed(v) => i128::from(v),
            Int::Unsigned(v) => i128::from(v),
        }
    }

    pub fn is_negative(self) -> bool {
        matches!(self, Int::Signed(v) if v < 0)
    }
}

impl From<i64> for Int {
    fn from(v: i64) -> Self {
        Int::Signed(v)
    }
}

impl From<u64> for Int {
    fn from(v: u64) -> Self {
        match i64::try_from(v) {
            Ok(v) => Int::Signed(v),
            Err(_) => Int::Unsigned(v),
        }
    }
}

/// What a field found at its key. Absence and null are separate states here;
/// collapsing them is the bug this type exists to prevent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Slot<T> {
    Absent,
    Null,
    Present(T),
}

impl<'a> Slot<&'a Value> {
    pub fn read(object: &'a BTreeMap<String, Value>, key: &str) -> Self {
        match object.get(key) {
            None => Slot::Absent,
            Some(Value::Null) => Slot::Null,
            Some(v) => Slot::Present(v),
        }
    }
}

impl crate::input::Input for Value {
    type Child<'a> = &'a Value;

    fn kind(&self) -> Kind {
        match self {
            Value::Null => Kind::Null,
            Value::Bool(_) => Kind::Bool,
            Value::Int(_) => Kind::Int,
            Value::Float(_) => Kind::Float,
            Value::String(_) => Kind::String,
            Value::Array(_) => Kind::Array,
            Value::Object(_) => Kind::Object,
            Value::IntTooWide => Kind::IntegerTooWide,
        }
    }

    fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    fn as_int(&self) -> Option<Int> {
        match self {
            Value::Int(n) => Some(*n),
            _ => None,
        }
    }

    fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(*f),
            // An integer is an acceptable float.
            Value::Int(n) => Some(n.as_i128() as f64),
            _ => None,
        }
    }

    fn as_str(&self) -> Option<std::borrow::Cow<'_, str>> {
        match self {
            Value::String(s) => Some(std::borrow::Cow::Borrowed(s)),
            _ => None,
        }
    }

    fn len(&self) -> usize {
        match self {
            Value::Array(items) => items.len(),
            Value::Object(map) => map.len(),
            _ => 0,
        }
    }

    fn item(&self, index: usize) -> Option<&Value> {
        match self {
            Value::Array(items) => items.get(index),
            _ => None,
        }
    }

    fn slot(&self, key: &str) -> Slot<&Value> {
        match self {
            Value::Object(map) => Slot::read(map, key),
            _ => Slot::Absent,
        }
    }

    fn each_key(&self, f: &mut dyn FnMut(&str)) {
        if let Value::Object(map) = self {
            for k in map.keys() {
                f(k);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn u64_above_i64_max_keeps_its_value() {
        let v = Int::from(u64::MAX);
        assert_eq!(v, Int::Unsigned(u64::MAX));
        assert_eq!(v.as_i128(), i128::from(u64::MAX));
    }

    #[test]
    fn the_max_safe_integer_boundary_is_exact() {
        let v = Int::from(9_007_199_254_740_993_u64);
        assert_eq!(v.as_i128(), 9_007_199_254_740_993_i128);
    }

    #[test]
    fn absent_and_null_do_not_collapse() {
        let mut o = BTreeMap::new();
        o.insert("null_key".to_string(), Value::Null);

        assert_eq!(Slot::read(&o, "null_key"), Slot::Null);
        assert_eq!(Slot::read(&o, "missing_key"), Slot::Absent);
    }
}

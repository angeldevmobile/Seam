use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(Int),
    Float(f64),
    String(String),
    Array(Vec<Value>),
    Object(BTreeMap<String, Value>),
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
pub enum Slot<'a> {
    Absent,
    Null,
    Present(&'a Value),
}

impl<'a> Slot<'a> {
    pub fn read(object: &'a BTreeMap<String, Value>, key: &str) -> Self {
        match object.get(key) {
            None => Slot::Absent,
            Some(Value::Null) => Slot::Null,
            Some(v) => Slot::Present(v),
        }
    }
}

impl Value {
    pub fn kind(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Int(_) => "integer",
            Value::Float(_) => "float",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
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

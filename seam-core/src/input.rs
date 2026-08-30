//! What the validator needs from a payload, and nothing more.
//!
//! Validation used to require a [`Value`], which meant every binding copied its
//! host's objects into one before a single rule ran. That copy was pure
//! overhead: allocated, walked once, dropped. Bindings now implement this trait
//! for their own runtime's objects and the copy disappears.
//!
//! [`Value`] still implements it, so building one by hand stays valid.
//!
//! [`Value`]: crate::value::Value

use std::borrow::Cow;

use crate::value::{Int, Slot};

/// What a value is, before asking what it holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Null,
    Bool,
    Int,
    Float,
    String,
    Array,
    Object,
    /// A real integer, but wider than 64 bits, so the model cannot hold it.
    ///
    /// Its own kind rather than `Foreign` because truncating here is the exact
    /// bug Seam exists to prevent, and the caller deserves to be told which of
    /// the two happened.
    IntegerTooWide,
    /// Something the model has no place for. A binding reports it rather than
    /// guessing.
    Foreign,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Null => "null",
            Kind::Bool => "bool",
            Kind::Int => "integer",
            Kind::Float => "float",
            Kind::String => "string",
            Kind::Array => "array",
            Kind::Object => "object",
            Kind::IntegerTooWide => "integer wider than 64 bits",
            Kind::Foreign => "unsupported value",
        }
    }
}

/// A payload the validator can read in place.
///
/// Accessors return `None` when the value is not of that kind, so a caller
/// never has to check twice. They are infallible on purpose: a binding that
/// cannot read its own object has a bug, not a validation failure.
pub trait Input {
    /// An element of an array, or the value at a key.
    /// Implementations should make this converge, usually to themselves or to a
    /// reference to themselves. A type whose child is a strictly new type on
    /// every level would make the validator recurse forever at compile time.
    type Child<'a>: Input
    where
        Self: 'a;

    fn kind(&self) -> Kind;

    fn as_bool(&self) -> Option<bool>;

    fn as_int(&self) -> Option<Int>;

    fn as_f64(&self) -> Option<f64>;

    fn as_str(&self) -> Option<Cow<'_, str>>;

    /// Elements for an array, keys for an object, zero otherwise.
    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn item(&self, index: usize) -> Option<Self::Child<'_>>;

    /// Reads a key without collapsing absence into null.
    fn slot(&self, key: &str) -> Slot<Self::Child<'_>>;

    /// Visits every key of an object, for the unknown-field check.
    fn each_key(&self, f: &mut dyn FnMut(&str));
}

impl<T: Input> Input for &T {
    type Child<'a>
        = T::Child<'a>
    where
        Self: 'a;

    fn kind(&self) -> Kind {
        (**self).kind()
    }

    fn as_bool(&self) -> Option<bool> {
        (**self).as_bool()
    }

    fn as_int(&self) -> Option<Int> {
        (**self).as_int()
    }

    fn as_f64(&self) -> Option<f64> {
        (**self).as_f64()
    }

    fn as_str(&self) -> Option<Cow<'_, str>> {
        (**self).as_str()
    }

    fn len(&self) -> usize {
        (**self).len()
    }

    fn item(&self, index: usize) -> Option<Self::Child<'_>> {
        (**self).item(index)
    }

    fn slot(&self, key: &str) -> Slot<Self::Child<'_>> {
        (**self).slot(key)
    }

    fn each_key(&self, f: &mut dyn FnMut(&str)) {
        (**self).each_key(f);
    }
}

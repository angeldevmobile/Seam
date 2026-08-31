use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Schema {
    pub types: BTreeMap<String, ObjectType>,
    /// Tagged unions, kept beside the object types rather than inside them.
    /// A union is a choice between objects, not a kind of object, and the two
    /// share one namespace: `declares` is what answers "is this name taken".
    pub unions: BTreeMap<String, UnionType>,
}

impl Schema {
    pub fn get(&self, name: &str) -> Option<&ObjectType> {
        self.types.get(name)
    }

    pub fn union(&self, name: &str) -> Option<&UnionType> {
        self.unions.get(name)
    }

    /// Whether the name is declared at all, as either an object or a union.
    pub fn declares(&self, name: &str) -> bool {
        self.types.contains_key(name) || self.unions.contains_key(name)
    }
}

/// A choice between object types, told apart by the value of one field.
///
/// The tag is always written down. A union that inferred which field decides,
/// or defaulted to a conventional name, would be guessing what the data means
/// — the same mistake as reading a naive datetime as local time.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UnionType {
    pub name: String,
    /// The key carrying the discriminant.
    pub tag: String,
    /// Declaration order, because it is the order the variants are listed in
    /// when a payload names none of them.
    pub variants: Vec<Variant>,
}

impl UnionType {
    pub fn variant(&self, tag: &str) -> Option<&Variant> {
        self.variants.iter().find(|v| v.tag == tag)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variant {
    /// The value of the tag field that selects this variant.
    pub tag: String,
    /// The object type it selects. Always a declared `schema`, never a union:
    /// a variant that were itself a union would need a second discriminant to
    /// resolve, and nothing in the payload says which one to read first.
    pub type_name: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ObjectType {
    pub name: String,
    /// Declaration order, because it is the order errors are reported in.
    pub fields: Vec<Field>,
    pub deny_unknown_fields: bool,
}

impl ObjectType {
    pub fn field(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.name == name)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub name: String,
    pub ty: Type,
    pub presence: Presence,
    pub rules: Vec<Rule>,
}

/// Two independent axes, not one. An absent key means "don't touch this field";
/// an explicit null means "clear it".
///
/// | `.seam`            | `optional` | `nullable` |
/// |--------------------|------------|------------|
/// | `String`           | false      | false      |
/// | `String?`          | false      | true       |
/// | `optional String`  | true       | false      |
/// | `optional String?` | true       | true       |
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Presence {
    pub optional: bool,
    pub nullable: bool,
}

impl Presence {
    pub const fn required() -> Self {
        Self { optional: false, nullable: false }
    }

    pub const fn nullable() -> Self {
        Self { optional: false, nullable: true }
    }

    pub const fn optional() -> Self {
        Self { optional: true, nullable: false }
    }

    pub const fn optional_nullable() -> Self {
        Self { optional: true, nullable: true }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    Bool,
    Int(IntType),
    Float,
    String,
    /// A calendar date. Never widened into an instant.
    Date,
    /// An instant with a mandatory UTC offset.
    DateTime,
    Enum(Vec<String>),
    /// An element has two states, a value or null, so only nullability applies
    /// to it. Absence is a property of a key, and an array has no keys.
    Array {
        item: Box<Type>,
        item_nullable: bool,
    },
    Object(Box<ObjectType>),
    Ref(String),
}

impl Type {
    pub fn kind(&self) -> &'static str {
        match self {
            Type::Bool => "bool",
            Type::Int(_) => "integer",
            Type::Float => "float",
            Type::String => "string",
            Type::Date => "date",
            Type::DateTime => "datetime",
            Type::Enum(_) => "enum",
            Type::Array { .. } => "array",
            Type::Object(_) | Type::Ref(_) => "object",
        }
    }
}

/// Width is part of the type because it is what makes a cross-language range
/// check possible at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntType {
    pub width: IntWidth,
    pub signed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntWidth {
    W8,
    W16,
    W32,
    W64,
}

impl IntType {
    pub fn range(self) -> (i128, i128) {
        let bits = match self.width {
            IntWidth::W8 => 8,
            IntWidth::W16 => 16,
            IntWidth::W32 => 32,
            IntWidth::W64 => 64,
        };
        if self.signed {
            let half = 1_i128 << (bits - 1);
            (-half, half - 1)
        } else {
            (0, (1_i128 << bits) - 1)
        }
    }

    pub fn name(self) -> &'static str {
        match (self.signed, self.width) {
            (true, IntWidth::W8) => "i8",
            (true, IntWidth::W16) => "i16",
            (true, IntWidth::W32) => "i32",
            (true, IntWidth::W64) => "i64",
            (false, IntWidth::W8) => "u8",
            (false, IntWidth::W16) => "u16",
            (false, IntWidth::W32) => "u32",
            (false, IntWidth::W64) => "u64",
        }
    }

    /// Whether the JS binding can use `number` rather than `bigint`.
    pub fn fits_js_number(self) -> bool {
        !matches!(self.width, IntWidth::W64)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Rule {
    MinLen(usize),
    MaxLen(usize),
    Range { min: i128, max: i128 },
    MinItems(usize),
    MaxItems(usize),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_ranges_are_exact_at_the_edges() {
        let u64t = IntType { width: IntWidth::W64, signed: false };
        assert_eq!(u64t.range(), (0, i128::from(u64::MAX)));

        let i32t = IntType { width: IntWidth::W32, signed: true };
        assert_eq!(i32t.range(), (i128::from(i32::MIN), i128::from(i32::MAX)));

        let u8t = IntType { width: IntWidth::W8, signed: false };
        assert_eq!(u8t.range(), (0, 255));
    }

    #[test]
    fn only_64_bit_integers_need_bigint_in_js() {
        assert!(IntType { width: IntWidth::W32, signed: true }.fits_js_number());
        assert!(!IntType { width: IntWidth::W64, signed: false }.fits_js_number());
    }

    #[test]
    fn presence_axes_are_independent() {
        assert_eq!(
            Presence::optional_nullable(),
            Presence { optional: true, nullable: true }
        );
        assert_ne!(Presence::optional(), Presence::nullable());
    }
}

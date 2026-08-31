//! `path` and `code` are public API in every binding. `code` is stable and
//! changes only on a major version; `message` is for humans, never parse it.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    Key(String),
    Index(usize),
}

impl fmt::Display for Segment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Segment::Key(k) => write!(f, "{k}"),
            Segment::Index(i) => write!(f, "[{i}]"),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Path(pub Vec<Segment>);

impl Path {
    pub fn render(&self) -> String {
        let mut out = String::new();
        for seg in &self.0 {
            match seg {
                Segment::Key(k) => {
                    if !out.is_empty() {
                        out.push('.');
                    }
                    out.push_str(k);
                }
                Segment::Index(i) => {
                    out.push('[');
                    out.push_str(&i.to_string());
                    out.push(']');
                }
            }
        }
        if out.is_empty() {
            "<root>".to_string()
        } else {
            out
        }
    }
}

impl fmt::Display for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.render())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Code {
    Required,
    NullNotAllowed,
    TypeMismatch,
    OutOfRange,
    UnsafeInteger,
    IntegerTooWide,
    NotFinite,
    TooShort,
    TooLong,
    TooFewItems,
    TooManyItems,
    NotInEnum,
    InvalidDate,
    InvalidDateTime,
    MissingTimezone,
    UnknownField,
    /// The tag named a variant the union does not declare.
    UnknownVariant,
    DepthExceeded,
    SizeExceeded,
    UnknownType,
}

impl Code {
    pub fn as_str(self) -> &'static str {
        match self {
            Code::Required => "required",
            Code::NullNotAllowed => "null_not_allowed",
            Code::TypeMismatch => "type_mismatch",
            Code::OutOfRange => "out_of_range",
            Code::UnsafeInteger => "unsafe_integer",
            Code::IntegerTooWide => "integer_too_wide",
            Code::NotFinite => "not_finite",
            Code::TooShort => "too_short",
            Code::TooLong => "too_long",
            Code::TooFewItems => "too_few_items",
            Code::TooManyItems => "too_many_items",
            Code::NotInEnum => "not_in_enum",
            Code::InvalidDate => "invalid_date",
            Code::InvalidDateTime => "invalid_datetime",
            Code::MissingTimezone => "missing_timezone",
            Code::UnknownField => "unknown_field",
            Code::UnknownVariant => "unknown_variant",
            Code::DepthExceeded => "depth_exceeded",
            Code::SizeExceeded => "size_exceeded",
            Code::UnknownType => "unknown_type",
        }
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    pub path: Path,
    pub code: Code,
    pub message: String,
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: {} ({})",
            self.path.render(),
            self.message,
            self.code
        )
    }
}

/// Every failure from one pass. Validation does not stop at the first, because
/// one issue per round trip is a bad way to debug a boundary.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidationError {
    pub issues: Vec<Issue>,
}

impl ValidationError {
    pub fn is_empty(&self) -> bool {
        self.issues.is_empty()
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.issues.as_slice() {
            [] => write!(f, "validation failed with no issues recorded"),
            [one] => write!(f, "{one}"),
            many => {
                write!(f, "{} validation issues:", many.len())?;
                for issue in many {
                    write!(f, "\n  - {issue}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for ValidationError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_render_in_the_documented_form() {
        let p = Path(vec![
            Segment::Key("user".into()),
            Segment::Key("tags".into()),
            Segment::Index(2),
        ]);
        assert_eq!(p.render(), "user.tags[2]");
        assert_eq!(Path::default().render(), "<root>");
    }

    #[test]
    fn codes_are_snake_case_and_stable() {
        assert_eq!(Code::NullNotAllowed.as_str(), "null_not_allowed");
        assert_eq!(Code::MissingTimezone.as_str(), "missing_timezone");
    }
}

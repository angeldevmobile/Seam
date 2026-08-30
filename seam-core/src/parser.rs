//! The `.seam` front end. **Not implemented yet** — this is the deliberate gap:
//! the semantics are built and tested first, because the mapping decisions are
//! the expensive-to-change part. Build a [`Schema`] directly until this lands.
//!
//! ```text
//! file        := declaration*
//! declaration := "schema" ident "{" field* "}"
//! field       := ident ":" presence type rule*
//! presence    := "optional"?
//! type        := ident "?"? | "[" type "]" | enum
//! enum        := "enum" "{" ident ("," ident)* ","? "}"
//! rule        := "@" ident "(" argument ")"
//! ```

use crate::schema::Schema;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.column, self.message)
    }
}

impl std::error::Error for ParseError {}

pub fn parse(_source: &str) -> Result<Schema, ParseError> {
    Err(ParseError {
        line: 1,
        column: 1,
        message: "the .seam parser is not implemented yet".to_string(),
    })
}

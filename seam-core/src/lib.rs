//! The Seam validation engine. Every binding is a translation layer over this
//! crate and contains no validation logic of its own. If a binding starts
//! growing rules, that is a leak here, to be fixed here.
//!
//! ```
//! use seam_core::{parser::parse, validate::validate, value::Value, Limits};
//! use std::collections::BTreeMap;
//!
//! let schema = parse("schema Signup { when: DateTime }")?;
//!
//! let mut payload = BTreeMap::new();
//! payload.insert("when".to_string(), Value::String("2026-08-29T14:30:00".into()));
//!
//! let err = validate(&schema, "Signup", &Value::Object(payload), Limits::DEFAULT)
//!     .unwrap_err();
//! assert_eq!(err.issues[0].code.as_str(), "missing_timezone");
//! # Ok::<(), seam_core::parser::ParseError>(())
//! ```

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod datetime;
pub mod error;
pub mod input;
pub mod limits;
pub mod parser;
pub mod schema;
pub mod validate;
pub mod value;

pub use error::{Code, Issue, Path, Segment, ValidationError};
pub use limits::Limits;
pub use parser::parse;
pub use schema::{Field, IntType, IntWidth, ObjectType, Presence, Rule, Schema, Type};
pub use validate::validate;
pub use value::{Int, Slot, Value};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

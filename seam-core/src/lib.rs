//! The Seam validation engine. Every binding is a translation layer over this
//! crate and contains no validation logic of its own — if a binding starts
//! growing rules, that is a leak here to be fixed here.
//!
//! [`parser`] is the one deliberate gap; schemas are built programmatically
//! until the `.seam` front end lands.
//!
//! ```
//! use seam_core::{
//!     limits::Limits,
//!     schema::{Field, ObjectType, Presence, Schema, Type},
//!     validate::validate,
//!     value::Value,
//! };
//! use std::collections::BTreeMap;
//!
//! let mut schema = Schema::default();
//! schema.types.insert(
//!     "Signup".into(),
//!     ObjectType {
//!         name: "Signup".into(),
//!         deny_unknown_fields: true,
//!         fields: vec![Field {
//!             name: "when".into(),
//!             ty: Type::DateTime,
//!             presence: Presence::required(),
//!             rules: vec![],
//!         }],
//!     },
//! );
//!
//! let mut payload = BTreeMap::new();
//! payload.insert("when".to_string(), Value::String("2026-08-29T14:30:00".into()));
//!
//! let err = validate(&schema, "Signup", &Value::Object(payload), Limits::DEFAULT)
//!     .unwrap_err();
//! assert_eq!(err.issues[0].code.as_str(), "missing_timezone");
//! ```

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod datetime;
pub mod error;
pub mod limits;
pub mod parser;
pub mod schema;
pub mod validate;
pub mod value;

pub use error::{Code, Issue, Path, Segment, ValidationError};
pub use limits::Limits;
pub use schema::{Field, IntType, IntWidth, ObjectType, Presence, Rule, Schema, Type};
pub use validate::validate;
pub use value::{Int, Slot, Value};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

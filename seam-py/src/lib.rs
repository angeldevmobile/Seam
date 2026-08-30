//! PyO3 binding. Translation only: lower a Python object into a
//! `seam_core::Value`, raise a `ValidationError` on the way back. No rule logic
//! belongs in this file.
//!
//! Not implemented yet. The two jobs it has to get right:
//!
//! - Lowering `int` without going through `float`, and reporting
//!   `integer_too_wide` above 64 bits rather than truncating.
//! - Distinguishing an absent key from `None`, which needs an `Absent` sentinel
//!   because Python has only one empty value.

fn _placeholder() {}

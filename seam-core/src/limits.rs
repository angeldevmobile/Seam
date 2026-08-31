//! Bounds on hostile input, enforced in the core so every binding inherits
//! them instead of each deriving its own defaults.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The validator recurses, so this is what protects the stack.
    pub max_depth: usize,
    pub max_items: usize,
    pub max_string_bytes: usize,
    pub max_object_keys: usize,
}

impl Limits {
    /// The highest `max_depth` the engine will honour, whatever it is handed.
    ///
    /// Both the JSON parser and the validator recurse, so depth is what stands
    /// between a hostile document and the stack. A stack overflow is not a
    /// panic that a binding can catch and turn into an error: it kills the
    /// process, taking every other request in flight with it. Measured, that
    /// starts happening somewhere between one and five thousand levels on the
    /// object path, so a caller raising `max_depth` freely could disable the
    /// one bound that is not recoverable.
    ///
    /// 256 is the value `PERMISSIVE` already used for trusted input, and it is
    /// far past any document a person meant to send. A limit that can be
    /// turned off until the process dies is not a limit.
    pub const MAX_DEPTH: usize = 256;

    pub const DEFAULT: Self = Self {
        max_depth: 64,
        max_items: 10_000,
        max_string_bytes: 1 << 20,
        max_object_keys: 1_000,
    };

    /// For trusted input. Depth stays bounded because the stack is.
    pub const PERMISSIVE: Self = Self {
        max_depth: 256,
        max_items: usize::MAX,
        max_string_bytes: usize::MAX,
        max_object_keys: usize::MAX,
    };
}

impl Limits {
    /// The limits actually used, with `max_depth` held under [`Self::MAX_DEPTH`].
    ///
    /// Applied by the engine at both entry points rather than by each binding,
    /// so no binding can hand the recursion a number the stack cannot take.
    #[must_use]
    pub const fn clamped(self) -> Self {
        Self {
            max_depth: if self.max_depth > Self::MAX_DEPTH {
                Self::MAX_DEPTH
            } else {
                self.max_depth
            },
            ..self
        }
    }
}

// Checked when the crate compiles, not when a test runs: a preset above the
// cap would be a contradiction in the engine's own configuration, and there is
// no reason to let it build.
const _: () = assert!(Limits::DEFAULT.max_depth <= Limits::MAX_DEPTH);
const _: () = assert!(Limits::PERMISSIVE.max_depth <= Limits::MAX_DEPTH);

impl Default for Limits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_depth_is_capped_however_it_arrives() {
        let reckless = Limits { max_depth: 1_000_000, ..Limits::DEFAULT };
        assert_eq!(reckless.clamped().max_depth, Limits::MAX_DEPTH);

        // Everything else is the caller's business: those bound memory, and
        // exceeding them is reported rather than fatal.
        assert_eq!(reckless.clamped().max_items, Limits::DEFAULT.max_items);

        // A limit below the cap is left alone.
        let tight = Limits { max_depth: 4, ..Limits::DEFAULT };
        assert_eq!(tight.clamped().max_depth, 4);
    }

    #[test]
    fn the_default_is_the_conservative_one() {
        assert_eq!(Limits::default(), Limits::DEFAULT);
        assert_eq!(
            Limits::DEFAULT.max_items.min(Limits::PERMISSIVE.max_items),
            Limits::DEFAULT.max_items
        );
    }
}

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

impl Default for Limits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_the_conservative_one() {
        assert_eq!(Limits::default(), Limits::DEFAULT);
        assert_eq!(
            Limits::DEFAULT.max_items.min(Limits::PERMISSIVE.max_items),
            Limits::DEFAULT.max_items
        );
    }
}

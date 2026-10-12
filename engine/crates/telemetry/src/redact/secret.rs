//! Wrappers whose `Debug` and `Display` never show the value.

use std::fmt;

/// The text printed in place of a protected value.
pub const PLACEHOLDER: &str = "«redacted»";

/// A value that must not appear in logs. `Debug` and `Display` print `«redacted»`; the only
/// way to read it is [`Redacted::expose`].
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Redacted<T>(T);

impl<T> Redacted<T> {
    pub fn new(value: T) -> Self {
        Self(value)
    }

    /// The protected value. Call sites should be the code that actually needs it.
    pub fn expose(&self) -> &T {
        &self.0
    }

    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> fmt::Debug for Redacted<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(PLACEHOLDER)
    }
}

impl<T> fmt::Display for Redacted<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(PLACEHOLDER)
    }
}

/// A secret string: [`Redacted`] over `String`.
pub type SecretString = Redacted<String>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_and_display_hide_the_value() {
        let s = Redacted::new("hunter2".to_owned());
        assert_eq!(format!("{s:?}"), PLACEHOLDER);
        assert_eq!(format!("{s}"), PLACEHOLDER);
        assert_eq!(s.expose(), "hunter2");
    }
}

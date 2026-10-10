//! `Name`: the one checked string a caller may join into a path under `$ROOT`.

use std::fmt;

use crate::error::invalid;
use crate::{Error, Result};

/// Max bytes in a name.
const NAME_MAX: usize = 64;

/// A name safe to join under `$ROOT`: 1 to 64 bytes of `a-z 0-9 _ -`.
///
/// Holding a `Name` proves the check ran, so `..`, `/` and `.` never reach a path.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Name(Box<str>);

impl TryFrom<&str> for Name {
    type Error = Error;

    /// Checks byte by byte. The set is ASCII, so a non-ASCII byte fails at once.
    fn try_from(s: &str) -> Result<Self> {
        let is_safe = (1..=NAME_MAX).contains(&s.len())
            && s.bytes()
                .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-'));
        if !is_safe {
            return Err(invalid("name", s));
        }
        Ok(Self(s.into()))
    }
}

impl AsRef<str> for Name {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_accepts_only_safe_slugs() {
        for ok in ["gruvbox", "a-1_b"] {
            assert!(Name::try_from(ok).is_ok(), "{ok}");
        }
        let long = "a".repeat(NAME_MAX + 1);
        for bad in ["", &long, "A", "a.b", "../x", "é"] {
            assert!(Name::try_from(bad).is_err(), "{bad}");
        }
    }
}

//! Ids that travel in the model context: a prefix plus lowercase base36.
//! `FromStr` is the only way in.

use std::error::Error;
use std::fmt;
use std::str::FromStr;

/// A string that is not a valid id of this kind. Never carries the input.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct IdError {
    kind: &'static str,
}

impl fmt::Display for IdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid id for {}", self.kind)
    }
}

impl Error for IdError {}

macro_rules! id {
    ($name:ident, $prefix:literal, $body_len:literal) => {
        #[doc = concat!("`", $prefix, "` plus ", $body_len, " lowercase base36 chars.")]
        #[derive(Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name([u8; $prefix.len() + $body_len]);

        impl $name {
            const PREFIX: &str = $prefix;
            const LEN: usize = $prefix.len() + $body_len;

            /// The id as text.
            ///
            /// # Panics
            ///
            /// Never: `FromStr` admits ASCII only.
            #[must_use]
            pub fn as_str(&self) -> &str {
                std::str::from_utf8(&self.0).expect("FromStr admits ASCII only")
            }
        }

        impl FromStr for $name {
            type Err = IdError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                let err = IdError {
                    kind: stringify!($name),
                };

                let bytes: [u8; Self::LEN] = s.as_bytes().try_into().map_err(|_| err)?;

                let (head, body) = bytes.split_at(Self::PREFIX.len());

                if head != Self::PREFIX.as_bytes() {
                    return Err(err);
                }

                if !body
                    .iter()
                    .all(|b| b.is_ascii_digit() || b.is_ascii_lowercase())
                {
                    return Err(err);
                }

                Ok(Self(bytes))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }
    };
}

id!(ToolId, "t", 7);
id!(SubSessionId, "ss", 6);
id!(SubagentId, "sa", 6);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_prints_back() {
        let id: ToolId = "t7q3gt5x".parse().unwrap();
        assert_eq!(id.to_string(), "t7q3gt5x");
        assert_eq!(format!("{id:?}"), "t7q3gt5x");
        assert!("ss2n4t6y".parse::<SubSessionId>().is_ok());
        assert!("sa1f3t5k".parse::<SubagentId>().is_ok());
    }

    #[test]
    fn rejects_bad_input() {
        let bad = [
            "",
            "t7q4x1b",
            "t7q4x1b29",
            "x7q4x1b2",
            "T7q4x1b2",
            "t7Q4x1b2",
            "t7q4-1b2",
            "t7q4x1é",
        ];
        for s in bad {
            assert!(s.parse::<ToolId>().is_err(), "{s}");
        }
    }
}

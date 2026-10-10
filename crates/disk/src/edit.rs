//! The pure core of `edit_file`: an exact byte replace with no I/O. Every byte outside a match is
//! kept as it is, so indentation and line endings survive. Tools runs several edits in memory, one
//! `replace` after the other on the last result, and writes once.

use memchr::memmem;

/// What `replace` did.
#[derive(Debug, PartialEq, Eq)]
pub enum ReplaceStatus {
    /// The edit applied.
    Done {
        /// The whole new content.
        bytes: Vec<u8>,
        /// How many matches were replaced.
        count: usize,
    },
    /// `old` is not in the content, or `old` is empty.
    NoMatch,
    /// `old` matches more than once and `all` is false; nothing changed.
    Ambiguous {
        /// How many times `old` matches.
        count: usize,
    },
}

/// Replaces the exact bytes `old` with `new` in `hay`. With `all` false, `old` must match once:
/// more matches give `Ambiguous`, so an edit never lands on the wrong copy. Matches never overlap.
#[must_use]
pub fn replace(hay: &[u8], old: &[u8], new: &[u8], all: bool) -> ReplaceStatus {
    if old.is_empty() {
        return ReplaceStatus::NoMatch;
    }
    let finder = memmem::Finder::new(old);
    // Without `all`, two matches already decide the answer; stop there on a big file.
    let limit = if all { usize::MAX } else { 2 };
    let starts: Vec<usize> = finder.find_iter(hay).take(limit).collect();
    match starts.len() {
        0 => ReplaceStatus::NoMatch,
        n if n > 1 && !all => ReplaceStatus::Ambiguous {
            count: finder.find_iter(hay).count(),
        },
        _ => ReplaceStatus::Done {
            bytes: splice(hay, &starts, old.len(), new),
            count: starts.len(),
        },
    }
}

/// Builds the new content in one pass: the gap before each match, then `new`, then the tail.
/// The capacity is exact: `k` matches each swap `old_len` bytes for `new.len()` bytes.
fn splice(hay: &[u8], starts: &[usize], old_len: usize, new: &[u8]) -> Vec<u8> {
    let k = starts.len();
    let mut out = Vec::with_capacity(hay.len() - k * old_len + k * new.len());
    let mut last = 0;
    for &s in starts {
        out.extend_from_slice(&hay[last..s]);
        out.extend_from_slice(new);
        last = s + old_len;
    }
    out.extend_from_slice(&hay[last..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_counts_and_refuses_ambiguity() {
        assert_eq!(
            replace(b"a b", b"b", b"c", false),
            ReplaceStatus::Done {
                bytes: b"a c".to_vec(),
                count: 1
            }
        );
        assert_eq!(
            replace(b"b b", b"b", b"c", false),
            ReplaceStatus::Ambiguous { count: 2 }
        );
        assert_eq!(
            replace(b"b b", b"b", b"cc", true),
            ReplaceStatus::Done {
                bytes: b"cc cc".to_vec(),
                count: 2
            }
        );
        assert_eq!(replace(b"abc", b"x", b"y", false), ReplaceStatus::NoMatch);
        assert_eq!(replace(b"abc", b"", b"y", false), ReplaceStatus::NoMatch);
    }
}

//! Errors for every disk operation.
use std::io;
use std::path::{Path, PathBuf};

/// Max char for a bad input kept in [`Error::Invalid`].
const GOT_MAX: usize = 64;

/// What failed in a disk operation.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// NO `$LAZYRIG_HOME` or `$XDG_DATA_HOME` or `$HOME` is set.
    #[error("no home dir: set $LAZYRIG_HOME or $HOME")]
    NoHome,

    /// An input failed its check: a name, a pattern or a glob.
    #[error("invalid {kind}: {got:?}")]
    Invalid {
        /// What was checked "name" "pattern" or "glob".
        kind: &'static str,
        /// The bad input, capped at 64 chars plus `...`.
        got: Box<str>,
    },

    /// A symlink points at nothing, nothing was written.
    #[error("dangling link: {}",.path.display())]
    Dangling {
        /// The link path.
        path: PathBuf,
    },

    /// An I/O call failed.
    #[error("io failed at {}",.path.display())]
    Io {
        /// the path the call touched.
        path: PathBuf,
        /// The OS error.
        source: io::Error,
    },
}

/// Results with [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

/// Wraps an I/O error with the path it touched.
pub(crate) fn io(path: &Path, source: io::Error) -> Error {
    Error::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Keeps the first `GOT_MAX` chars of s, then `...` when cut
pub(crate) fn cap(s: &str) -> Box<str> {
    match s.char_indices().nth(GOT_MAX) {
        Some((i, _)) => format!("{}...", &s[..i]).into(),
        None => s.into(),
    }
}

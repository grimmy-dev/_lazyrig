//! Errors for every disk operation. Each variant names what failed, so a caller can wrap it in its
//! own error and log it without guessing.

use std::io;
use std::path::{Path, PathBuf};

/// Max chars of a bad input kept in [`Error::Invalid`].
const GOT_MAX: usize = 64;

/// What failed in a disk operation.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Neither `$LAZYRIG_HOME` nor `$HOME` is set.
    #[error("no home dir: set $LAZYRIG_HOME or $HOME")]
    NoHome,

    /// An input failed its check.
    #[error("invalid {kind}: {got:?}")]
    Invalid {
        /// What was checked: `"name"`, `"pattern"` or `"glob"`.
        kind: &'static str,
        /// The bad input, capped at 64 chars plus `...`, so a huge input never fills a log line.
        got: Box<str>,
    },

    /// A symlink points at nothing; nothing was written.
    #[error("dangling link: {}", .path.display())]
    Dangling {
        /// The link path.
        path: PathBuf,
    },

    /// An I/O call failed.
    #[error("io failed at {}", .path.display())]
    Io {
        /// The path the call touched.
        path: PathBuf,
        /// The OS error.
        source: io::Error,
    },
}

/// Result with [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

/// Wraps an I/O error with the path it touched.
pub(crate) fn io(path: &Path, source: io::Error) -> Error {
    Error::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Turns "not found" into `None` and any other I/O error into [`Error::Io`]. Most disk calls treat a
/// missing path as a normal answer, not a failure; this keeps that rule in one place.
pub(crate) fn found<T>(path: &Path, result: io::Result<T>) -> Result<Option<T>> {
    match result {
        Ok(v) => Ok(Some(v)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io(path, e)),
    }
}

/// Builds [`Error::Invalid`] with the input capped.
pub(crate) fn invalid(kind: &'static str, got: &str) -> Error {
    Error::Invalid {
        kind,
        got: cap(got),
    }
}

/// Keeps the first `GOT_MAX` chars of `s`, then `...` when cut. Cuts on a char boundary.
fn cap(s: &str) -> Box<str> {
    match s.char_indices().nth(GOT_MAX) {
        Some((i, _)) => format!("{}...", &s[..i]).into(),
        None => s.into(),
    }
}

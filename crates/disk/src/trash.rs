//! Every delete a user may want back goes here: sessions, skill dirs, the model's delete tool.
//! It moves the entry to the OS trash and never removes it.

use std::path::Path;
use std::{fs, io};

use crate::Result;
use crate::error::io;

/// Moves a file or a dir to the OS trash (freedesktop on Linux, Trash on macOS).
/// A missing path is `Ok`, so a second call is harmless. There is no fallback to `remove_*`.
/// Blocking; may copy across file systems.
///
/// # Errors
/// [`crate::Error::Io`] when the path cannot be read or the move fails; nothing is removed.
pub fn put(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(io(path, e)),
        Ok(_) => {}
    }
    // `::trash` is the crate; a bare `trash` could read as this module.
    ::trash::delete(path).map_err(|e| io(path, io::Error::other(e)))
}

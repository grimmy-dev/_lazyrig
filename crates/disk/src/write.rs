//! Crash-safe writes: a crash leaves the old file or the new file, never half of one.

use std::ffi::OsStr;
use std::fs::{self, File, Permissions};
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::error::{found, io};
use crate::temp::temp_path;
use crate::{Error, Result};

/// Permission bits of a mode, without the file-type bits.
const MODE_BITS: u32 = 0o7777;

/// Replaces `path` with `bytes` through a temp file and a rename.
/// A link is followed and survives; an existing file keeps its mode; missing parent dirs are created.
/// Blocking: a caller on the runtime runs it inside one `spawn_blocking`.
///
/// # Errors
/// [`Error::Dangling`] for a link with no target, nothing written;
/// [`Error::Io`] when a step fails, with the temp file removed.
pub fn atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let target = resolve(path)?;
    let (dir, name) = split(&target)?;
    let mode = found(&target, fs::metadata(&target))?.map(|m| m.permissions().mode() & MODE_BITS);
    fs::create_dir_all(dir).map_err(|e| io(dir, e))?;
    let temp = temp_path(dir, name);
    // Every step that can fail once the temp exists, so the cleanup below covers them all.
    let written = write_temp(&temp, bytes, mode)
        .and_then(|()| fs::rename(&temp, &target))
        .and_then(|()| sync_dir(dir));
    if let Err(e) = written {
        // The first error is the one worth reporting; a failed cleanup must not hide it.
        let _ = fs::remove_file(&temp);
        return Err(io(&target, e));
    }
    Ok(())
}

/// Creates `path` with `bytes` only when nothing is there, so a user's file is never overwritten.
/// The file appears under its real name whole, through a temp file and a hard link.
/// Returns `true` when it wrote the file, `false` when any entry (even a dangling link) was there.
/// Blocking, like [`atomic`].
///
/// # Errors
/// [`Error::Io`] when a step fails, with the temp file removed.
pub fn create_new(path: &Path, bytes: &[u8]) -> Result<bool> {
    // Checked first so the common "already there" case costs one stat, not a temp write and fsync.
    if found(path, fs::symlink_metadata(path))?.is_some() {
        return Ok(false);
    }
    let (dir, name) = split(path)?;
    fs::create_dir_all(dir).map_err(|e| io(dir, e))?;
    let temp = temp_path(dir, name);
    if let Err(e) = write_temp(&temp, bytes, None) {
        let _ = fs::remove_file(&temp);
        return Err(io(path, e));
    }
    // The kernel refuses the link when `path` exists, so a racing writer gets `false`.
    let linked = fs::hard_link(&temp, path);
    let _ = fs::remove_file(&temp);
    match linked {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Ok(false),
        Err(e) if is_no_hard_link(&e) => return write_direct(path, dir, bytes),
        Err(e) => return Err(io(path, e)),
    }
    sync_dir(dir).map_err(|e| io(dir, e))?;
    Ok(true)
}

/// The real file to write: a link's target, or `path` itself when it is not a link or does not exist yet.
fn resolve(path: &Path) -> Result<PathBuf> {
    let is_link =
        found(path, fs::symlink_metadata(path))?.is_some_and(|m| m.file_type().is_symlink());
    if !is_link {
        return Ok(path.to_path_buf());
    }
    found(path, path.canonicalize())?.ok_or_else(|| Error::Dangling {
        path: path.to_path_buf(),
    })
}

/// Creates `temp` (never over an existing file), writes `bytes`, applies `mode` and syncs it to disk.
fn write_temp(temp: &Path, bytes: &[u8], mode: Option<u32>) -> io::Result<()> {
    let mut file = File::create_new(temp)?;
    file.write_all(bytes)?;
    // Set after create: the umask may have cleared bits the old file had.
    if let Some(mode) = mode {
        file.set_permissions(Permissions::from_mode(mode))?;
    }
    file.sync_all()
}

/// Syncs a dir, so a rename inside it survives a power loss.
fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

/// Splits a file path into its dir and its name; a bare `f.txt` gets the dir `.`.
fn split(path: &Path) -> Result<(&Path, &OsStr)> {
    let Some(name) = path.file_name() else {
        return Err(io(path, io::ErrorKind::InvalidInput.into()));
    };
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    Ok((dir, name))
}

/// True when a file system has no hard links: `Unsupported`, or `EPERM` on some FUSE mounts.
/// A real permission problem then fails again in [`write_direct`] with its own error.
fn is_no_hard_link(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::Unsupported | io::ErrorKind::PermissionDenied
    )
}

/// Fallback for file systems with no hard links: create and write in place, with the same steps
/// as a temp. Only here can a half file carry the real name; a failed write removes it, since we
/// just made it.
fn write_direct(path: &Path, dir: &Path, bytes: &[u8]) -> Result<bool> {
    match write_temp(path, bytes, None).and_then(|()| sync_dir(dir)) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(false),
        Err(e) => {
            let _ = fs::remove_file(path);
            Err(io(path, e))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use tempfile::TempDir;

    #[test]
    fn atomic_replaces_and_creates_parents() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("a/b/c.txt");
        atomic(&file, b"one").unwrap();
        atomic(&file, b"two").unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"two");
        assert_eq!(fs::read_dir(tmp.path().join("a/b")).unwrap().count(), 1);
    }

    #[test]
    fn atomic_writes_through_link_and_refuses_dangling_links() {
        let tmp = TempDir::new().unwrap();
        let real = tmp.path().join("real.lua");
        let link = tmp.path().join("link.lua");
        fs::write(&real, "old").unwrap();
        symlink(&real, &link).unwrap();
        atomic(&link, b"new").unwrap();
        assert!(link.symlink_metadata().unwrap().is_symlink());
        assert_eq!(fs::read(&real).unwrap(), b"new");

        let dangling = tmp.path().join("gone.lua");
        symlink(tmp.path().join("missing"), &dangling).unwrap();
        assert!(matches!(
            atomic(&dangling, b"x"),
            Err(Error::Dangling { .. })
        ));
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 3);
    }

    #[test]
    fn atomic_keeps_mode() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("f");
        fs::write(&file, "a").unwrap();
        fs::set_permissions(&file, Permissions::from_mode(0o640)).unwrap();
        atomic(&file, b"b").unwrap();
        assert_eq!(file.metadata().unwrap().permissions().mode() & 0o777, 0o640);
    }

    #[test]
    fn create_new_writes_once_and_never_clobbers() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("init.lua");
        assert!(create_new(&path, b"one").unwrap());
        assert!(!create_new(&path, b"two").unwrap());
        assert_eq!(fs::read(&path).unwrap(), b"one");
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 1);
    }
}

//! Crash-safe writes: a crash leaves the old file or the new file, never half of one.

use std::ffi::OsStr;
use std::fs::{self, File, Permissions};
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::io;
use crate::{Error, Result};

/// Permission bits of a mode, without the file-type bits.
const MODE_BITS: u32 = 0o7777;

/// Makes temp names unique inside one process; the pid makes them unique across processes.
static COUNTER: AtomicU64 = AtomicU64::new(0);

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
    let mode = match fs::metadata(&target) {
        Ok(m) => Some(m.permissions().mode() & MODE_BITS),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(io(&target, e)),
    };
    fs::create_dir_all(dir).map_err(|e| io(dir, e))?;
    let temp = temp_for(dir, name);
    if let Err(e) = replace(&temp, &target, dir, bytes, mode) {
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
    match fs::symlink_metadata(path) {
        Ok(_) => return Ok(false),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(io(path, e)),
    }
    let (dir, name) = split(path)?;
    fs::create_dir_all(dir).map_err(|e| io(dir, e))?;
    let temp = temp_for(dir, name);
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
        Err(e) if is_no_hard_link(&e) => return write_direct(path, bytes),
        Err(e) => return Err(io(path, e)),
    }
    sync_dir(dir).map_err(|e| io(dir, e))?;
    Ok(true)
}

/// The real file to write: a link's target, or `path` itself when it is not a link or does not exist yet.
fn resolve(path: &Path) -> Result<PathBuf> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(path.to_path_buf()),
        Err(e) => return Err(io(path, e)),
        Ok(meta) if !meta.file_type().is_symlink() => return Ok(path.to_path_buf()),
        Ok(_) => {}
    }
    match path.canonicalize() {
        Ok(target) => Ok(target),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Err(Error::Dangling {
            path: path.to_path_buf(),
        }),
        Err(e) => Err(io(path, e)),
    }
}

/// A temp path beside the target, `<name>.tmp.<pid>.<n>`, so the rename stays on one file system.
fn temp_for(dir: &Path, name: &OsStr) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut temp = name.to_os_string();
    temp.push(format!(".tmp.{}.{n}", process::id()));
    dir.join(temp)
}

/// Writes the temp, renames it over the target and syncs the dir: every step that can fail once the temp exists.
fn replace(
    temp: &Path,
    target: &Path,
    dir: &Path,
    bytes: &[u8],
    mode: Option<u32>,
) -> io::Result<()> {
    write_temp(temp, bytes, mode)?;
    fs::rename(temp, target)?;
    sync_dir(dir)
}

/// Creates `temp` (never an existing file), writes `bytes`, applies `mode` and syncs to disk.
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

/// Fallback for file systems with no hard links: create and write in place.
/// Only here can a half file carry the real name; a failed write removes it, since we just made it.
fn write_direct(path: &Path, bytes: &[u8]) -> Result<bool> {
    let mut file = match File::create_new(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Ok(false),
        Err(e) => return Err(io(path, e)),
    };
    if let Err(e) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        let _ = fs::remove_file(path);
        return Err(io(path, e));
    }
    Ok(true)
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

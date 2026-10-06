//! Where lazyrig's home is, which project a cwd is, and the one safe name.

use std::env;
use std::ffi::OsString;
use std::fs::{self, Permissions};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::error::io;
use crate::{Error, Result};

mod name;
pub use name::Name;

/// Dir under `$HOME` when `$LAZYRIG_HOME` is not set.
const DOT_DIR: &str = ".lazyrig";
/// Mode for `$ROOT`: owner only.
const PRIVATE_MODE: u32 = 0o700;
/// Group and other permission bits.
const GROUP_OTHER_BITS: u32 = 0o077;
/// Max bytes of the readable part of a project key.
const KEY_READABLE_MAX: usize = 64;
/// Bytes of the sha256 digest kept in a project key.
const HASH_BYTES: usize = 4;
/// Lowercase hex digits, indexed by nibble.
const HEX: &[u8; 16] = b"0123456789abcdef";

/// The project a cwd belongs to.
#[derive(Clone, Debug)]
pub struct Project {
    /// `$ROOT`.
    pub home: PathBuf,
    /// The canonical cwd.
    pub dir: PathBuf,
    /// Readable tail of `dir` plus 8 hex of its sha256, at most 73 bytes.
    /// `/home/user/Desktop/_lazyrig` gives `home-user-Desktop-_lazyrig-<8 hex>`.
    pub key: Box<str>,
}

/// Creates `$ROOT` and its parents, then sets it to `0700` when group or other bits are set.
/// A symlinked `$ROOT` is followed, so the target holds the data.
///
/// # Errors
/// [`Error::NoHome`] when neither `$LAZYRIG_HOME` nor `$HOME` is set;
/// [`Error::Io`] when a create, stat or chmod fails.
pub fn ensure_home() -> Result<PathBuf> {
    let home = home()?;
    fs::create_dir_all(&home).map_err(|e| io(&home, e))?;
    let mode = fs::metadata(&home)
        .map_err(|e| io(&home, e))?
        .permissions()
        .mode();
    if mode & GROUP_OTHER_BITS != 0 {
        fs::set_permissions(&home, Permissions::from_mode(PRIVATE_MODE))
            .map_err(|e| io(&home, e))?;
    }
    Ok(home)
}

/// Resolves the project for `cwd`. Creates nothing.
///
/// # Errors
/// [`Error::NoHome`] when neither `$LAZYRIG_HOME` nor `$HOME` is set;
/// [`Error::Io`] when `cwd` cannot be canonicalized.
pub fn project(cwd: &Path) -> Result<Project> {
    let home = home()?;
    let dir = cwd.canonicalize().map_err(|e| io(cwd, e))?;
    let key = key_of(&dir);
    Ok(Project { home, dir, key })
}

/// `$ROOT` from the real env. The only `std::env` read in the crate.
fn home() -> Result<PathBuf> {
    home_from(env::var_os("LAZYRIG_HOME"), env::var_os("HOME"))
}

/// `$LAZYRIG_HOME` when set and not empty, else `$HOME/.lazyrig`, else `NoHome`.
/// Takes values, so tests never touch the real env.
fn home_from(lazyrig: Option<OsString>, home: Option<OsString>) -> Result<PathBuf> {
    if let Some(dir) = lazyrig.filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    match home.filter(|v| !v.is_empty()) {
        Some(home) => Ok(PathBuf::from(home).join(DOT_DIR)),
        None => Err(Error::NoHome),
    }
}

/// Builds the project key. sha256 output never changes between toolchains,
/// so a project keeps its key, and its sessions, across upgrades.
fn key_of(dir: &Path) -> Box<str> {
    let bytes = dir.as_os_str().as_bytes();
    let digest = Sha256::digest(bytes);
    let readable: String = bytes
        .strip_prefix(b"/")
        .unwrap_or(bytes)
        .iter()
        .map(|&b| match b {
            b'/' => '-',
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-' => char::from(b),
            _ => '_',
        })
        .collect();
    // Every byte is ASCII now, so any byte index is a char boundary.
    let tail = &readable[readable.len().saturating_sub(KEY_READABLE_MAX)..];

    let mut key = String::with_capacity(KEY_READABLE_MAX + 1 + HASH_BYTES * 2);
    match tail.as_bytes().first() {
        None => key.push_str("root"),
        // A leading `.` hides the dir, a leading `-` reads as a flag in a shell.
        Some(b'.' | b'-') => {
            key.push('_');
            key.push_str(&tail[1..]);
        }
        Some(_) => key.push_str(tail),
    }
    key.push('-');
    for &b in &digest[..HASH_BYTES] {
        key.push(char::from(HEX[usize::from(b >> 4)]));
        key.push(char::from(HEX[usize::from(b & 0x0f)]));
    }
    key.into_boxed_str()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[expect(clippy::unnecessary_wraps, reason = "keeps test calls short")]
    fn os(s: &str) -> Option<OsString> {
        Some(s.into())
    }

    #[test]
    fn home_prefers_lazyrig_then_dot_lazyrig() {
        assert_eq!(
            home_from(os("/lr"), os("/h")).unwrap(),
            PathBuf::from("/lr")
        );
        assert_eq!(
            home_from(os(""), os("/h")).unwrap(),
            PathBuf::from("/h/.lazyrig")
        );
    }

    #[test]
    fn home_without_env_is_no_home() {
        assert!(matches!(home_from(None, None), Err(Error::NoHome)));
        assert!(matches!(home_from(os(""), os("")), Err(Error::NoHome)));
    }

    #[test]
    fn key_is_readable_tail_plus_hash() {
        let key = key_of(Path::new("/home/user/Desktop/_lazyrig"));
        let (readable, hash) = key.rsplit_once('-').unwrap();
        assert_eq!(readable, "home-user-Desktop-_lazyrig");
        assert_eq!(hash.len(), 8);
        assert!(hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
    }

    #[test]
    fn key_handles_long_odd_and_root_paths() {
        let long = format!("/{}", "a".repeat(200));
        assert_eq!(key_of(Path::new(&long)).len(), 64 + 1 + 8);
        assert!(key_of(Path::new("/tmp/café x")).starts_with("tmp-caf___x-"));
        assert!(key_of(Path::new("/.hidden")).starts_with("_hidden-"));
        assert!(key_of(Path::new("/")).starts_with("root-"));
    }
}

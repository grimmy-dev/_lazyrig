//! Owns the temp-file name that `write` uses, and cleans up the temps a crash left behind.
//! The binary sweeps `$ROOT` at boot; config sweeps its lua dirs.

use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use crate::Result;
use crate::error::found;

/// Age at which a temp counts as stale. A write takes milliseconds, so a live writer's temp is never this old.
const TEMP_MAX_AGE: Duration = Duration::from_hours(1);
/// The tag between the name and the pid. Built and parsed only in this file, so the two never drift.
const TAG: &str = "tmp";

/// Makes temp names unique inside one process; the pid makes them unique across processes.
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh temp path beside the target: `<name>.tmp.<pid>.<n>`. Same dir, so a rename over the
/// target stays on one file system and stays atomic.
pub(crate) fn temp_path(dir: &Path, name: &OsStr) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut temp = name.to_os_string();
    temp.push(format!(".{TAG}.{}.{n}", process::id()));
    dir.join(temp)
}

/// Removes `<stem>.tmp.<pid>.<n>` files in `dir` older than one hour. Not recursive.
/// Only names from the listing decide what goes; a failed remove skips that entry.
/// Returns how many it removed, for the caller to log.
///
/// # Errors
/// [`crate::Error::Io`] when `dir` exists but cannot be listed. A missing `dir` is `Ok(0)`.
pub fn sweep(dir: &Path) -> Result<usize> {
    let Some(entries) = found(dir, fs::read_dir(dir))? else {
        return Ok(0);
    };
    let now = SystemTime::now();
    let mut removed = 0;
    for entry in entries.flatten() {
        if !is_temp_name(&entry.file_name()) {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        // An mtime in the future is an error here, so it never counts as old.
        let is_old = now
            .duration_since(modified)
            .is_ok_and(|age| age > TEMP_MAX_AGE);
        if is_old && fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

/// True for `<stem>.tmp.<digits>.<digits>`, the name [`temp_path`] builds.
/// Split from the right, so a stem with dots stays whole.
fn is_temp_name(name: &OsStr) -> bool {
    let mut parts = name.as_bytes().rsplitn(4, |&b| b == b'.');
    let (Some(n), Some(pid), Some(tmp), Some(stem)) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    let is_digits = |s: &[u8]| !s.is_empty() && s.iter().all(u8::is_ascii_digit);
    tmp == TAG.as_bytes() && is_digits(pid) && is_digits(n) && !stem.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use tempfile::TempDir;

    #[test]
    fn sweep_removes_only_old_temps() {
        let tmp = TempDir::new().unwrap();
        let old = tmp.path().join("x.tmp.1.0");
        let fresh = tmp.path().join("y.tmp.2.0");
        let other = tmp.path().join("z.tmp");
        File::create(&old)
            .unwrap()
            .set_modified(SystemTime::now() - 2 * TEMP_MAX_AGE)
            .unwrap();
        File::create(&fresh).unwrap();
        File::create(&other).unwrap();
        assert_eq!(sweep(tmp.path()).unwrap(), 1);
        assert!(!old.exists() && fresh.exists() && other.exists());
    }
}

//! The project tree behind `list_dir`: a sorted walk that hides dot and gitignored entries unless
//! asked. `search` walks with the same rules, so a list and a search always see the same files.

use std::fs;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

use crate::Result;
use crate::error::io;

/// One entry in a tree.
#[derive(Debug, Clone)]
pub struct Entry {
    /// Full path, starting with `root`.
    pub path: PathBuf,
    /// Levels below `root`; a direct child is `1`.
    pub depth: usize,
    /// True for a dir. A link is never followed, so a link to a dir is `false`.
    pub is_dir: bool,
    /// Size in bytes; `0` for a dir.
    pub len: u64,
}

/// Walks `root` down to `depth` levels, sorted by name, and hands each entry to `f` until it
/// returns `Break`. `hidden` false hides dot entries and anything gitignored; true shows both.
/// The root itself is not passed, and an entry that cannot be read is skipped.
/// Blocking: a caller on the runtime runs it inside one `spawn_blocking`.
///
/// # Errors
/// [`crate::Error::Io`] when `root` does not exist or cannot be read.
pub fn tree(
    root: &Path,
    depth: usize,
    hidden: bool,
    mut f: impl FnMut(Entry) -> ControlFlow<()>,
) -> Result<()> {
    let walk = walker(root, hidden)?
        .max_depth(Some(depth))
        .sort_by_file_name(Ord::cmp)
        .build();
    for item in walk {
        let Ok(e) = item else {
            continue;
        };
        // Skip the root here, not with `min_depth(1)`: that also skips reading the root's
        // `.gitignore`, and ignored entries come back.
        if e.depth() == 0 {
            continue;
        }
        let is_dir = e.file_type().is_some_and(|t| t.is_dir());
        let len = if is_dir {
            0
        } else {
            e.metadata().map_or(0, |m| m.len())
        };
        let entry = Entry {
            depth: e.depth(),
            is_dir,
            len,
            // Moves the path out of `e`, so it goes last.
            path: e.into_path(),
        };
        if f(entry).is_break() {
            break;
        }
    }
    Ok(())
}

/// The one place the walk rules live. `ignore` reads its flags as "skip this", the opposite of
/// our `hidden`, hence every `!hidden`. Ignore files above `root` still apply, so a subdir keeps
/// the repo's `.gitignore`. Links are never followed: a loop never ends, and a link out of the
/// project would leak files.
pub(crate) fn walker(root: &Path, hidden: bool) -> Result<WalkBuilder> {
    // Without this check a missing root gives an empty walk, not an error.
    fs::metadata(root).map_err(|e| io(root, e))?;
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(!hidden)
        .git_ignore(!hidden)
        .git_global(!hidden)
        .git_exclude(!hidden)
        .ignore(!hidden)
        .parents(true)
        .follow_links(false)
        // Git objects are never worth a walk, even with `hidden` on.
        .filter_entry(|e| e.file_name() != ".git");
    Ok(builder)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn repo() -> TempDir {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        for dir in [".git", "src/deep", "target"] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        fs::write(root.join(".gitignore"), "target/\n").unwrap();
        for file in [".env", "src/a.rs", "src/deep/b.rs", "target/t.rs"] {
            fs::write(root.join(file), "").unwrap();
        }
        tmp
    }

    fn names(root: &Path, depth: usize, hidden: bool) -> Vec<String> {
        let mut out = Vec::new();
        tree(root, depth, hidden, |e| {
            out.push(e.path.strip_prefix(root).unwrap().display().to_string());
            ControlFlow::Continue(())
        })
        .unwrap();
        out
    }

    #[test]
    fn tree_hides_dot_and_ignored_entries() {
        let tmp = repo();
        assert_eq!(
            names(tmp.path(), 9, false),
            ["src", "src/a.rs", "src/deep", "src/deep/b.rs"]
        );
        let all = names(tmp.path(), 9, true);
        assert!(all.contains(&".env".to_string()));
        assert!(all.contains(&"target/t.rs".to_string()));
        assert!(!all.contains(&".git".to_string()));
    }

    #[test]
    fn tree_sorts_limits_depth_and_breaks() {
        let tmp = repo();
        assert_eq!(
            names(tmp.path(), 1, true),
            [".env", ".gitignore", "src", "target"]
        );
        let mut calls = 0;
        tree(tmp.path(), 9, true, |_| {
            calls += 1;
            if calls == 2 {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        })
        .unwrap();
        assert_eq!(calls, 2);
    }
}

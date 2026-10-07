//! The engine behind `search_files`: a parallel regex search on ripgrep's crates. It keeps at most
//! `max` lines but counts every match, so the footer can say how much it left out.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use grep_regex::RegexMatcherBuilder;
use grep_searcher::{BinaryDetection, Searcher, SearcherBuilder, Sink, SinkContext, SinkMatch};
use ignore::WalkState;
use ignore::overrides::OverrideBuilder;

use crate::error::cap;
use crate::walk::walker;
use crate::{Error, Result};

/// Size and DFA limit for a compiled pattern, so a hostile pattern fails at build time instead
/// of eating memory.
const REGEX_LIMIT: usize = 1024 * 1024;

/// One search request.
pub struct Grep<'a> {
    /// A regex. Smart case: all lowercase matches any case, one capital makes it exact.
    pub pattern: &'a str,
    /// One glob such as `*.rs`; only matching files are searched.
    pub glob: Option<&'a str>,
    /// Same as [`crate::walk::tree`]: false skips dot and gitignored entries.
    pub hidden: bool,
    /// Lines kept before and after each match.
    pub context: u32,
    /// Match lines kept. Counting goes on past it.
    pub max: u32,
    /// The caller's cancel check, asked once per file.
    pub stop: &'a (dyn Fn() -> bool + Sync),
}

/// One kept line.
#[derive(Debug)]
pub struct Line {
    /// Line number, from 1.
    pub n: u64,
    /// The bytes without the `\n`; a `\r` stays.
    pub text: Box<[u8]>,
    /// True for a context line, false for a match.
    pub is_context: bool,
}

/// The kept lines of one file, in line order.
#[derive(Debug)]
pub struct FileHits {
    /// The file.
    pub path: PathBuf,
    /// Its kept lines.
    pub lines: Vec<Line>,
}

/// The result of a search.
#[derive(Debug)]
pub struct Found {
    /// Files with kept lines, sorted by path.
    pub files: Vec<FileHits>,
    /// Every match, kept or not.
    pub matches: u64,
    /// Every file with a match, kept or not.
    pub files_matched: u64,
    /// True when `matches` went past `max`, so some lines were left out.
    pub capped: bool,
}

/// Searches every file under `root` in parallel, with the same hidden and ignore rules as
/// [`crate::walk::tree`]. Binary files are skipped. Workers keep lines until the shared count
/// passes `max` and only count after that, so memory stays bounded on any repo.
/// Blocking: a caller on the runtime runs it inside one `spawn_blocking`.
///
/// # Errors
/// [`Error::Invalid`] for a bad pattern or glob; [`Error::Io`] when `root` cannot be read.
pub fn grep(root: &Path, g: &Grep) -> Result<Found> {
    let regex = RegexMatcherBuilder::new()
        .case_smart(true)
        .size_limit(REGEX_LIMIT)
        .dfa_size_limit(REGEX_LIMIT)
        .build(g.pattern)
        .map_err(|_| Error::Invalid {
            kind: "pattern",
            got: cap(g.pattern),
        })?;
    let mut builder = walker(root, g.hidden)?;
    if let Some(glob) = g.glob {
        let overrides = OverrideBuilder::new(root)
            .add(glob)
            .and_then(|o| o.build())
            .map_err(|_| Error::Invalid {
                kind: "glob",
                got: cap(glob),
            })?;
        builder.overrides(overrides);
    }

    let hits = Mutex::new(Vec::new());
    let matches = AtomicU64::new(0);
    let files_matched = AtomicU64::new(0);
    let max = u64::from(g.max);
    let context = g.context as usize;

    // The outer closure runs once per thread: each worker owns its regex and searcher, and
    // takes the lock only to push a finished file.
    builder.build_parallel().run(|| {
        let regex = regex.clone();
        let mut searcher = SearcherBuilder::new()
            .binary_detection(BinaryDetection::quit(0))
            .line_number(true)
            .before_context(context)
            .after_context(context)
            .build();
        // Borrow here so `move` takes the references, not the shared state.
        let (hits, matches, files_matched) = (&hits, &matches, &files_matched);
        Box::new(move |item| {
            if (g.stop)() {
                return WalkState::Quit;
            }
            let Ok(entry) = item else {
                return WalkState::Continue;
            };
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                return WalkState::Continue;
            }
            let mut sink = Collect {
                matches,
                max,
                lines: Vec::new(),
                had_match: false,
            };
            // A file that vanished or cannot be read is skipped, like in `tree`.
            if searcher
                .search_path(&regex, entry.path(), &mut sink)
                .is_err()
            {
                return WalkState::Continue;
            }
            if sink.had_match {
                files_matched.fetch_add(1, Ordering::Relaxed);
            }
            if !sink.lines.is_empty() {
                // A poisoned lock still holds good data: take it rather than panic.
                hits.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(FileHits {
                        path: entry.into_path(),
                        lines: sink.lines,
                    });
            }
            WalkState::Continue
        })
    });

    let mut files = hits.into_inner().unwrap_or_else(PoisonError::into_inner);
    // Workers finish in any order; one sort at the end gives a stable output.
    files.sort_unstable_by(|a, b| a.path.cmp(&b.path));
    let matches = matches.into_inner();
    Ok(Found {
        files,
        matches,
        files_matched: files_matched.into_inner(),
        capped: matches > max,
    })
}

/// Collects one file's lines while the shared match count is under `max`.
struct Collect<'a> {
    matches: &'a AtomicU64,
    max: u64,
    lines: Vec<Line>,
    had_match: bool,
}

impl Sink for Collect<'_> {
    type Error = io::Error;

    fn matched(&mut self, _: &Searcher, m: &SinkMatch<'_>) -> io::Result<bool> {
        self.had_match = true;
        // `fetch_add` returns the count before this match.
        let seen = self.matches.fetch_add(1, Ordering::Relaxed) + 1;
        if seen <= self.max {
            self.lines.push(Line {
                n: m.line_number().unwrap_or(0),
                text: line_text(m.bytes()),
                is_context: false,
            });
        }
        Ok(true)
    }

    fn context(&mut self, _: &Searcher, c: &SinkContext<'_>) -> io::Result<bool> {
        if self.matches.load(Ordering::Relaxed) < self.max {
            self.lines.push(Line {
                n: c.line_number().unwrap_or(0),
                text: line_text(c.bytes()),
                is_context: true,
            });
        }
        Ok(true)
    }
}

/// The line bytes without the `\n`. A `\r` stays, the same rule as [`crate::read::lines`].
fn line_text(bytes: &[u8]) -> Box<[u8]> {
    bytes.strip_suffix(b"\n").unwrap_or(bytes).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn run(root: &Path, pattern: &str, glob: Option<&str>, max: u32) -> Result<Found> {
        grep(
            root,
            &Grep {
                pattern,
                glob,
                hidden: false,
                context: 0,
                max,
                stop: &|| false,
            },
        )
    }

    #[test]
    fn grep_finds_sorted_with_smart_case_and_skips_binary() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("b.rs"), "fn main() {}\n").unwrap();
        fs::write(tmp.path().join("a.rs"), "x\nfn main() {}\nFoo\nfoo\n").unwrap();
        fs::write(tmp.path().join("bin.dat"), b"fn main\0").unwrap();

        let f = run(tmp.path(), "fn main", None, 100).unwrap();
        assert_eq!(f.matches, 2);
        let paths: Vec<_> = f
            .files
            .iter()
            .map(|h| h.path.file_name().unwrap())
            .collect();
        assert_eq!(paths, ["a.rs", "b.rs"]);
        assert_eq!(f.files[0].lines[0].n, 2);

        assert_eq!(run(tmp.path(), "Foo", None, 100).unwrap().matches, 1);
        assert_eq!(run(tmp.path(), "foo", None, 100).unwrap().matches, 2);
    }

    #[test]
    fn grep_caps_lines_but_counts_all() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("f.txt"), "hit\n".repeat(5)).unwrap();
        let f = run(tmp.path(), "hit", None, 2).unwrap();
        assert_eq!(f.files[0].lines.len(), 2);
        assert_eq!(f.matches, 5);
        assert!(f.capped);
    }

    #[test]
    fn grep_rejects_bad_pattern_and_glob() {
        let tmp = TempDir::new().unwrap();
        assert!(matches!(
            run(tmp.path(), "(", None, 10),
            Err(Error::Invalid {
                kind: "pattern",
                ..
            })
        ));
        assert!(matches!(
            run(tmp.path(), "x", Some("["), 10),
            Err(Error::Invalid { kind: "glob", .. })
        ));
    }
}

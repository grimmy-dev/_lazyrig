//! The read side of the file tools: a stamp for the read-before-write rule, a sniff to refuse
//! binary, and a line stream that never loads the whole file.

use std::fs::{self, File};
use std::io::{self, Read};
use std::ops::ControlFlow;
use std::path::Path;
use std::time::SystemTime;

use crate::Result;
use crate::error::io;

/// Bytes `sniff` looks at.
const SNIFF_BYTES: usize = 8 * 1024;
/// Read buffer for `lines`.
const LINE_BUF: usize = 64 * 1024;

/// A cheap fingerprint of a file. Tools keeps the one from the last read and compares it before a
/// write: a different stamp means someone else changed the file.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Stamp {
    /// Last modified time.
    pub mtime: SystemTime,
    /// Length in bytes.
    pub len: u64,
}

/// What a file holds, guessed from its first bytes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sniff {
    /// No magic bytes and no NUL: safe to show as text.
    Text,
    /// An image the model can see: `"png"`, `"jpeg"`, `"gif"` or `"webp"`.
    Image(&'static str),
    /// A PDF.
    Pdf,
    /// Any other binary; tools refuses it.
    Binary,
}

/// Reads the mtime and length of `path` with one `metadata` call.
///
/// # Errors
/// [`crate::Error::Io`] when `path` cannot be read or the OS has no mtime.
pub fn stamp(path: &Path) -> Result<Stamp> {
    let meta = fs::metadata(path).map_err(|e| io(path, e))?;
    let mtime = meta.modified().map_err(|e| io(path, e))?;
    Ok(Stamp {
        mtime,
        len: meta.len(),
    })
}

/// Reads at most the first 8 KiB of `path` and says what it holds.
///
/// # Errors
/// [`crate::Error::Io`] when `path` cannot be opened or read.
pub fn sniff(path: &Path) -> Result<Sniff> {
    let file = File::open(path).map_err(|e| io(path, e))?;
    let mut head = Vec::with_capacity(SNIFF_BYTES);
    file.take(SNIFF_BYTES as u64)
        .read_to_end(&mut head)
        .map_err(|e| io(path, e))?;
    Ok(classify(&head))
}

/// Streams `path` one line at a time and never loads the whole file. Lines before `from` are
/// skipped (`0` and `1` both mean the first line); `f` gets the line number and the bytes without
/// the `\n`, and a `\r` stays so a CRLF file is visible. `f` returns `Break` to stop.
/// Blocking: a caller on the runtime runs it inside one `spawn_blocking`.
///
/// # Errors
/// [`crate::Error::Io`] when `path` cannot be opened or read.
pub fn lines(
    path: &Path,
    from: u64,
    mut f: impl FnMut(u64, &[u8]) -> ControlFlow<()>,
) -> Result<()> {
    let mut file = File::open(path).map_err(|e| io(path, e))?;
    let mut buf = vec![0u8; LINE_BUF];
    // Holds the start of a line that crosses the end of the buffer; the only copy in the loop.
    let mut carry: Vec<u8> = Vec::new();
    let mut n: u64 = 0;
    loop {
        let read = match file.read(&mut buf) {
            Ok(0) => break,
            Ok(k) => k,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(io(path, e)),
        };
        let chunk = &buf[..read];
        let mut start = 0;
        for end in memchr::memchr_iter(b'\n', chunk) {
            n += 1;
            let line = if carry.is_empty() {
                &chunk[start..end]
            } else {
                carry.extend_from_slice(&chunk[start..end]);
                &carry[..]
            };
            if n >= from && f(n, line).is_break() {
                return Ok(());
            }
            carry.clear();
            start = end + 1;
        }
        carry.extend_from_slice(&chunk[start..]);
    }
    // The last line when the file does not end with `\n`.
    if !carry.is_empty() {
        n += 1;
        if n >= from {
            let _ = f(n, &carry);
        }
    }
    Ok(())
}

/// The pure decision behind `sniff`: magic bytes first, then a NUL means binary.
fn classify(head: &[u8]) -> Sniff {
    match head {
        [0x89, b'P', b'N', b'G', ..] => Sniff::Image("png"),
        [0xFF, 0xD8, 0xFF, ..] => Sniff::Image("jpeg"),
        [b'G', b'I', b'F', b'8', ..] => Sniff::Image("gif"),
        // Bytes 4..8 are the RIFF size; the format tag follows.
        [b'R', b'I', b'F', b'F', _, _, _, _, rest @ ..] if rest.starts_with(b"WEBP") => {
            Sniff::Image("webp")
        }
        [b'%', b'P', b'D', b'F', ..] => Sniff::Pdf,
        _ if memchr::memchr(0, head).is_some() => Sniff::Binary,
        _ => Sniff::Text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn all_lines(path: &Path) -> Vec<(u64, Vec<u8>)> {
        let mut out = Vec::new();
        lines(path, 1, |n, l| {
            out.push((n, l.to_vec()));
            ControlFlow::Continue(())
        })
        .unwrap();
        out
    }

    #[test]
    fn classify_checks_magic_then_nul() {
        assert_eq!(classify(b"\x89PNG\r\n\x1a\n"), Sniff::Image("png"));
        assert_eq!(classify(b"RIFF\0\0\0\0WEBPVP8 "), Sniff::Image("webp"));
        assert_eq!(classify(b"%PDF-1.7"), Sniff::Pdf);
        let mut bin = vec![b'a'; 200];
        bin[100] = 0;
        assert_eq!(classify(&bin), Sniff::Binary);
        assert_eq!(classify(b"fn main() {}"), Sniff::Text);
    }

    #[test]
    fn lines_returns_window_and_stops() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("f");
        let text = (1..=300)
            .map(|i| format!("line {i}\n"))
            .collect::<Vec<_>>()
            .concat();
        fs::write(&path, text).unwrap();

        let mut got = Vec::new();
        lines(&path, 10, |n, l| {
            got.push((n, l.to_vec()));
            if got.len() == 4 {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        })
        .unwrap();
        let want: Vec<_> = (10..=13)
            .map(|i| (i, format!("line {i}").into_bytes()))
            .collect();
        assert_eq!(got, want);
    }

    #[test]
    fn lines_handles_long_crlf_last_and_empty() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("f");
        let big = "a".repeat(200 * 1024);
        fs::write(&path, format!("{big}\nx\r\nlast")).unwrap();

        let got = all_lines(&path);
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].1.len(), 200 * 1024);
        assert_eq!(got[1].1, b"x\r");
        assert_eq!(got[2], (3, b"last".to_vec()));

        let empty = tmp.path().join("empty");
        fs::write(&empty, "").unwrap();
        assert!(all_lines(&empty).is_empty());
    }
}

//! How fast the tree walk and the parallel grep run on a generated repo: 5 000 files of 200 lines
//! in 50 dirs, with 10 dirs gitignored.

use std::fmt::Write as _;
use std::fs;
use std::hint::black_box;
use std::ops::ControlFlow;
use std::path::Path;
use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};
use lazyrig_disk::search::{Grep, grep};
use lazyrig_disk::walk::tree;
use tempfile::TempDir;

/// Dirs in the repo; the first `IGNORED` are in `.gitignore`.
const DIRS: usize = 50;
/// Gitignored dirs.
const IGNORED: usize = 10;
/// Files per dir.
const FILES: usize = 100;
/// Lines per file.
const LINES: usize = 200;
/// Files per dir that hold one `NEEDLE` line.
const NEEDLE_FILES: usize = 3;

/// Builds the repo. Visible: 40 dirs × 100 files, 120 `NEEDLE` lines, and 8 000 lines matching
/// `compute\(\d*77\)` (lines 77 and 177 of each file).
fn make_repo(root: &Path) {
    fs::create_dir(root.join(".git")).unwrap();
    let mut ignore = String::new();
    for d in 0..IGNORED {
        writeln!(ignore, "d{d}/").unwrap();
    }
    fs::write(root.join(".gitignore"), ignore).unwrap();

    let mut body = String::new();
    for i in 0..LINES {
        writeln!(body, "let value_{i} = compute({i});").unwrap();
    }
    let with_needle = format!("{body}NEEDLE\n");

    for d in 0..DIRS {
        let dir = root.join(format!("d{d}"));
        fs::create_dir(&dir).unwrap();
        for f in 0..FILES {
            let text = if f < NEEDLE_FILES {
                &with_needle
            } else {
                &body
            };
            fs::write(dir.join(format!("f{f}.rs")), text).unwrap();
        }
    }
}

fn bench(c: &mut Criterion) {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    make_repo(root);

    let never = || false;
    let literal = Grep {
        pattern: "NEEDLE",
        glob: None,
        hidden: false,
        context: 0,
        max: 200,
        stop: &never,
    };
    let regex = Grep {
        pattern: r"compute\(\d*77\)",
        ..literal
    };

    let mut g = c.benchmark_group("search");
    // A pass takes 30-100 ms: 20 samples of the slowest case need about 20 s.
    g.sample_size(20);
    g.measurement_time(Duration::from_secs(20));

    g.bench_function("tree_depth_10", |b| {
        b.iter(|| {
            let mut n = 0u64;
            tree(black_box(root), 10, false, |_| {
                n += 1;
                ControlFlow::Continue(())
            })
            .unwrap();
            n
        });
    });

    g.bench_function("grep_literal_120_hits", |b| {
        b.iter(|| {
            let found = grep(black_box(root), &literal).unwrap();
            assert_eq!(found.matches, 120);
            found
        });
    });

    g.bench_function("grep_regex_8000_hits_max_200", |b| {
        b.iter(|| {
            let found = grep(black_box(root), &regex).unwrap();
            assert_eq!(found.matches, 8000);
            found
        });
    });

    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);

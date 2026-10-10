//! How fast `read` sniffs a file, serves a `read_file` window and streams every line.
//! Inputs are made at bench start in a temp dir, so nothing large lives in git.

use std::fs;
use std::hint::black_box;
use std::ops::ControlFlow;
use std::time::Duration;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use lazyrig_disk::read::{lines, sniff};
use tempfile::TempDir;

/// One line of the big file: 79 bytes and a `\n`, close to a line of source code.
const LINE: &[u8] =
    b"let value = compute(input, &config).map_err(Error::from)?; // a typical line!!!\n";
/// Size of the big file.
const BIG: usize = 100 * 1024 * 1024;
/// Lines a `read_file` window asks for.
const WINDOW: u64 = 2000;

fn bench(c: &mut Criterion) {
    // Page cache serves the reads either way, so a RAM-backed temp dir is fine here.
    let tmp = TempDir::new().unwrap();
    let big = tmp.path().join("big.txt");
    let big_lines = BIG / LINE.len();
    fs::write(&big, LINE.repeat(big_lines)).unwrap();
    let small = tmp.path().join("small.txt");
    fs::write(&small, LINE.repeat(1024 * 1024 / LINE.len())).unwrap();

    let mut g = c.benchmark_group("read");
    // A 100 MiB pass takes about 100 ms: 10 samples in 10 s give a stable median without a long run.
    g.sample_size(10);
    g.measurement_time(Duration::from_secs(10));

    g.bench_function("sniff_1mib", |b| {
        b.iter(|| sniff(black_box(&small)).unwrap());
    });

    // Scans to the middle, then reads 2000 lines and stops: the cost of a `read_file` window.
    g.bench_function("lines_window_2000_mid", |b| {
        b.iter(|| {
            let mut n = 0u64;
            lines(black_box(&big), big_lines as u64 / 2, |_, _| {
                n += 1;
                if n == WINDOW {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            })
            .unwrap();
            n
        });
    });

    // Set last: throughput sticks to every bench after it in the group, and only this one reads
    // the whole file.
    g.throughput(Throughput::Bytes((big_lines * LINE.len()) as u64));
    g.bench_function("lines_count_100mib", |b| {
        b.iter(|| {
            let mut n = 0u64;
            lines(black_box(&big), 1, |_, _| {
                n += 1;
                ControlFlow::Continue(())
            })
            .unwrap();
            n
        });
    });

    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);

//! What a crash-safe write costs. Two fsyncs dominate, so the number depends on the disk.
//! It runs under `target/`, on the disk `$ROOT` lives on: tmpfs has no real fsync and would lie.

use std::hint::black_box;
use std::time::Duration;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use lazyrig_disk::write::{atomic, create_new};

/// A small config or source file.
const SMALL: usize = 4 * 1024;
/// A big generated file.
const LARGE: usize = 1024 * 1024;

fn bench(c: &mut Criterion) {
    // Cargo sets this for benches: a dir inside `target/`, on the real disk.
    let tmp = tempfile::Builder::new()
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap();
    let small = vec![b'x'; SMALL];
    let large = vec![b'x'; LARGE];

    let mut g = c.benchmark_group("write");
    // Each write waits on the disk, so samples are slow and noisy: 10 samples over 10 s.
    g.sample_size(10);
    g.measurement_time(Duration::from_secs(10));

    let path = tmp.path().join("atomic_small");
    g.bench_function("atomic_4k", |b| {
        b.iter(|| atomic(&path, black_box(&small)).unwrap());
    });

    let path = tmp.path().join("atomic_large");
    g.bench_function("atomic_1m", |b| {
        b.iter(|| atomic(&path, black_box(&large)).unwrap());
    });

    // `create_new` only writes to a free path, so each run gets a new name. The name is built in
    // the setup closure, which `iter_batched` keeps out of the timing.
    let mut i = 0u64;
    g.bench_function("create_new_4k", |b| {
        b.iter_batched(
            || {
                i += 1;
                tmp.path().join(format!("new_{i}"))
            },
            |path| assert!(create_new(&path, black_box(&small)).unwrap()),
            BatchSize::SmallInput,
        );
    });

    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);

//! What an exact replace costs on a 1 MiB file: one unique match, and 10 000 matches with `all`.
//! Pure and in memory, so this is the edit itself with no I/O.

use std::hint::black_box;
use std::time::Duration;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use lazyrig_disk::edit::replace;

/// Size of the haystack.
const SIZE: usize = 1024 * 1024;
/// Matches in the `all` case.
const MANY: usize = 10_000;

fn bench(c: &mut Criterion) {
    // One `NEEDLE` in the middle of filler: the common edit, which must still scan the whole file
    // to prove there is no second match.
    let mut one = vec![b'x'; SIZE];
    one[SIZE / 2..][..6].copy_from_slice(b"NEEDLE");

    // `ab` at the start of every 100-byte block: 10 000 matches spread over the file.
    let mut block = [b'x'; 100];
    block[..2].copy_from_slice(b"ab");
    let many = block.repeat(MANY);

    let mut g = c.benchmark_group("edit");
    // The `all` case takes about 1 ms: 100 samples need more than the default 5 s.
    g.measurement_time(Duration::from_secs(10));
    g.throughput(Throughput::Bytes(one.len() as u64));
    g.bench_function("replace_one_1mib", |b| {
        b.iter(|| replace(black_box(&one), b"NEEDLE", b"pin", false));
    });
    // 10 000 blocks of 100 bytes is 1 MB, a little under 1 MiB; the throughput uses the real size.
    g.throughput(Throughput::Bytes(many.len() as u64));
    g.bench_function("replace_all_10k_1mb", |b| {
        b.iter(|| replace(black_box(&many), b"ab", b"cd", true));
    });
    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);

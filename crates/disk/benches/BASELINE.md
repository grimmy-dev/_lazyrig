# disk bench baseline

Numbers from one machine. Compare a change against this table on the same machine only, never across
machines.

| | |
| --- | --- |
| CPU | Intel Core i5-10210U @ 1.60GHz (laptop, 4 cores) |
| RAM | 7.6 GiB |
| OS | Linux 7.2.7-200.fc44.x86_64 (Fedora 44) |
| File system | btrfs on LUKS (`target/`, where `write` runs); `/tmp` is tmpfs |
| Toolchain | rustc 1.98.1 (48a229cea 2026-09-01) |
| Date | 2026-10-07 |

This laptop is noisy: the same code moved up to 20% between two runs, from turbo and thermal state. A change
under about 20% here is noise unless it repeats.

Median from criterion's `time` line. Saved as the criterion baseline `main`. To check a change, I save a
second baseline and compare the two with [critcmp](https://github.com/BurntSushi/critcmp), which prints one
row per bench and hides changes under 20%:

```sh
cargo bench -p lazyrig-disk -- --save-baseline new
critcmp main new -t 20
cargo bench -p lazyrig-disk -- --save-baseline main   # make the new numbers the baseline
```

Charts with every sample are in `target/criterion/report/index.html`.

## read

| Case | Median | Throughput |
| --- | --- | --- |
| `sniff` 1 MiB file | 8.0 µs | |
| `lines` window: from the middle of 100 MiB, 2000 lines | 44.3 ms | |
| `lines` count every line, 100 MiB | 97.9 ms | 1.02 GiB/s |

A window costs the scan to `from`: line 650 000 costs about 45 ms, line 100 costs microseconds.

## edit

| Case | Median | Throughput |
| --- | --- | --- |
| `replace` one match, 1 MiB | 340 µs | 2.87 GiB/s |
| `replace` `all`, 10 000 matches, 1 MB | 829 µs | 1.12 GiB/s |

## write

| Case | Median |
| --- | --- |
| `atomic` 4 KiB | 9.7 ms |
| `atomic` 1 MiB | 13.2 ms |
| `create_new` 4 KiB | 11.3 ms |

Two fsyncs (the file and its dir) are most of the cost, so the size barely matters.

## search

Generated repo: 50 dirs × 100 files × 200 lines, 10 dirs gitignored, so 4 000 files are searched.

| Case | Median |
| --- | --- |
| `walk::tree` depth 10, 4 040 entries | 33.2 ms |
| `grep` literal, 120 hits | 28.6 ms |
| `grep` regex `compute\(\d*77\)`, 8 000 hits, `max = 200` | 78.7 ms |

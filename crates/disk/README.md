# lazyrig-disk

Every file mechanic lazyrig needs, in one crate: where home is, crash-safe writes, the OS trash, the temp
sweep, and the read, edit, walk and search behind the model's file tools.

I keep it stateless and boring on purpose. Free functions over sync std, no async and no logging, so a
caller on the runtime wraps one call in `spawn_blocking` and logs errors with its own rules.

## Modules

| Module | What it does |
| --- | --- |
| `paths` | `ensure_home` makes `$ROOT` with mode `0700`; `project` turns a cwd into a stable key; `Name` is the one checked string you may join under `$ROOT` |
| `write` | `atomic` replaces a file through a temp file and a rename; `create_new` writes only when nothing is there |
| `temp` | `sweep` removes temp files a crash left behind, older than one hour |
| `trash` | `put` moves a file or dir to the OS trash and never removes it |
| `read` | `stamp` for the read-before-write check, `sniff` to tell text from image, PDF or binary, `lines` to stream a file without loading it |
| `edit` | `replace` does an exact byte replace with no I/O; more than one match without `all` is refused |
| `walk` | `tree` gives a sorted walk that hides dot and gitignored entries unless asked |
| `search` | `grep` runs a parallel regex search on ripgrep's crates, keeps at most `max` lines and counts every match |

`$ROOT` is `$LAZYRIG_HOME` when set, otherwise `~/.lazyrig`.

## Rules it keeps

- A crash leaves the old file or the new file, never half of one.
- A delete a user may want back goes to the trash, with no fallback to `remove_*`.
- A link is followed on write and survives; a dangling link is an error and nothing is written.
- `walk` and `search` share one walker, so a list and a search always see the same files.
- A bad name, pattern or glob fails at the boundary. A hostile regex fails at build time, not in memory.
- Every error names what failed and the path it touched. A bad input is capped at 64 chars in the error.

## Use

```rust
use lazyrig_disk::{paths, write};

let home = paths::ensure_home()?;
write::atomic(&home.join("config.lua"), b"return {}\n")?;
```

Errors are one enum, `lazyrig_disk::Error`: `NoHome`, `Invalid`, `Dangling` and `Io`.

## Test

```sh
cargo test -p lazyrig-disk
```

Unix only (Linux, macOS).

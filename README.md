# _lazyrig

A lazyvim-inspired multi-agent harness in Rust. Lazy, customizable and minimal, shaped around how I work.

I want to get better at Rust and understand agent orchestration end to end, from context and memory to what ends up on screen. I am a big TUI fan and use LazyVim every day, so I am building the harness I want to open every day, and learning by doing it properly.

## Status

Early. The first crate, `disk`, is done: home and project paths, crash-safe writes, the OS trash and the
temp-file sweep. The other crates come next, one at a time.

## Layout

```
core      types, no I/O
ports     traits over core
store, config, tools, mcp   adapters; each calls disk for file mechanics
binary    wiring
disk      beside the adapters: stateless file functions over std
```

## Build

Unix only (Linux, macOS). You need `rustup`; `rust-toolchain.toml` pins the compiler and rustup picks it up.

```sh
cargo build
cargo test
cargo clippy --all-targets
cargo fmt --check
```

## Data

lazyrig keeps its data in `~/.lazyrig`, mode `0700`. Set `LAZYRIG_HOME` to use another dir, which is how I
keep a dev build away from my real sessions.

## License

Apache-2.0. See [LICENSE](LICENSE).

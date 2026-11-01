# lazyrig-core

The words every lazyrig crate shares: ids, scopes and timing types. No I/O, no async, no traits and no
workspace dependency, so it is the one crate every other crate can name without pulling anything in.

It grows only when a crate needs a new word. Today that is what the ticker needs.

## Modules

| Module | What it holds |
| --- | --- |
| `id` | `ToolId`, `SubSessionId`, `SubagentId`: a short prefix plus lowercase base36, made by one `id!` macro |
| `scope` | `Scope`, the owner of a call, a record, a tick or a tab, and its one log spelling |
| `time` | `TickId`, `TickKind`, `WatchKind`, `Tick`, `Budget`, `Timeouts`, `Expiry` |
| `pulse` | `Pulse` and `Hold`: the activity signal of a running task |

## Rules it keeps

- `FromStr` is the only way to make an id from a string. It checks the prefix, the length and the
  alphabet once, and every later use trusts the type.
- An id error names the kind of id and never carries the input.
- Ids are fixed-size arrays, so they are `Copy` and never allocate.
- `Pulse` is the one type with behaviour. A beat is one clock read and one atomic store, and a released
  `Hold` beats before it lets go, so a reader never sees a stale beat after a hold.

## Test

```sh
cargo test -p lazyrig-core
```

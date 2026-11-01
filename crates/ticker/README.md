# lazyrig-ticker

Time for every running thing in lazyrig: how long it has run, plus a hard and a stale timer on the
things that can be killed. When a timer runs out, the ticker fires the task's own `CancellationToken`.
The task kills its own work.

Nothing ticks. A tick is a start time in one map, and elapsed is worked out when someone reads it. One
task sleeps on tokio's timer wheel and wakes only when a deadline is due, so an idle process costs no
wakes at all.

## Modules

| Module | What it does |
| --- | --- |
| `table` | the one map of running ticks, the id counter, and the frontend's `snapshot` rows |
| `budget` | picks a kind's `Budget` and applies the model's clamp |
| `watch` | the timer state of one watched tick, and `TickWatch`, the handle the task holds |
| `reaper` | `check`, the pure timing rule, and `run`, the one task that fires spent timers |

## Rules it keeps

- A beat is one clock read and one atomic store. It never wakes the reaper: at a deadline the reaper
  reads the last beat and either fires or waits again.
- The model's `timeout` can lower the configured ceiling, never raise it.
- Every budget is clamped to 30 days when a timer is armed, so a typo in lua cannot overflow the
  deadline math or the timer wheel.
- A hold pauses the stale timer while a subagent waits on the user. The hard timer keeps running.
- Dropping a watch removes its tick and disarms it without firing. Every cleanup is safe to repeat.
- `stop_all` removes rows and never fires a token. The caller's token tree is the one kill path.
- `snapshot` gives rows in start order, with one `now` for every row and no allocation once the
  caller's buffer has grown.

## Use

```rust
use lazyrig_core::{scope::Scope, time::{Timeouts, WatchKind}};
use lazyrig_ports::Clock;
use lazyrig_ticker::Ticker;
use tokio_util::sync::CancellationToken;

let ticker = Ticker::new(Timeouts::default);
let token = CancellationToken::new();
let watch = ticker.watch(WatchKind::Shell, Scope::Session, None, None, None, token.clone());

let pulse = watch.pulse();   // beat once per output chunk
pulse.beat();

let took = watch.finish();   // or drop it: the same cleanup, without the elapsed
```

`watch` must run inside a tokio runtime: the first call spawns the reaper. The ticker logs nothing yet.
One line per fired timer comes with the binary.

## Test

```sh
cargo test -p lazyrig-ticker
```

The timing rule is a pure function, so its tests build instants by addition and never sleep. The
wiring tests use real time with budgets of tens of milliseconds, because tokio's paused clock does not
move `std::time::Instant`.

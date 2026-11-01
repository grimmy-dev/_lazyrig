# lazyrig-ports

The traits the application needs from the outside, one per outside thing. The agent names these and never
an adapter, so the dependency list itself says the agent cannot touch a file, a socket or a clock except
through a port.

A port grows when the application needs something, never because an adapter wants to show something.
Today it holds one:

| Port | Implemented by | What the application uses it for |
| --- | --- | --- |
| `Clock`, `Watch` | `lazyrig-ticker` | start and stop ticks, and watch a task that can be killed |

## Rules it keeps

- Every trait is object safe, so the agent holds `Arc<dyn Clock>` and a test holds a fake.
- A port names core types only. It never names a type from an adapter.

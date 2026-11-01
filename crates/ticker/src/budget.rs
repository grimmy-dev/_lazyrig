//! Picks a kind's budget and applies the model's clamp.

use lazyrig_core::time::{Budget, Timeouts, WatchKind};
use std::time::Duration;

/// The budget for one timer. The model's timeout can lower the configured ceiling, never
/// raise it. A kind with no ceiling (`Background`, `McpServer`) takes the model's as is.
pub(crate) fn budget(
    timeouts: &Timeouts,
    kind: WatchKind,
    model_timeout: Option<Duration>,
) -> Budget {
    let configured = match kind {
        WatchKind::Shell => timeouts.shell,
        WatchKind::Background => timeouts.background,
        WatchKind::McpCall => timeouts.mcp_call,
        WatchKind::Subagent => timeouts.subagent,
        WatchKind::McpServer => timeouts.mcp_server,
    };
    let timeout = match (model_timeout, configured.timeout) {
        (Some(asked), Some(ceiling)) => Some(asked.min(ceiling)),
        _ => model_timeout.or(configured.timeout),
    };
    Budget {
        timeout,
        ..configured
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn model_lowers_ceiling_never_raises_it() {
        let timeouts = Timeouts::default();
        let rows = [
            // (kind, model timeout, hard timer)
            (WatchKind::Shell, Some(secs(60)), Some(secs(60))), // lowers the ceiling
            (WatchKind::Shell, Some(secs(9000)), Some(secs(1800))), // clamped to it
            (WatchKind::Shell, None, Some(secs(1800))),         // no model timeout: the ceiling
            (WatchKind::Background, None, None),                // no ceiling at all
            (WatchKind::Background, Some(secs(7200)), Some(secs(7200))), // the model's only
        ];
        for (kind, model_timeout, want) in rows {
            assert_eq!(
                budget(&timeouts, kind, model_timeout).timeout,
                want,
                "{kind:?} {model_timeout:?}"
            );
        }
        assert_eq!(
            budget(&timeouts, WatchKind::Shell, None).idle_timeout,
            secs(300)
        );
    }
}

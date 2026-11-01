//! The timing vocabulary every crate shares: tick ids and kinds.

use crate::{id::ToolId, scope::Scope};
use std::{num::NonZeroU64, sync::Arc, time::Duration};

/// One running tick. Process-local, from a counter. Never in a prompt, never on disk.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct TickId(NonZeroU64);

impl TickId {
    /// Wrap a counter value. The ticker owns the counter, so it alone mints ids.
    #[must_use]
    pub const fn new(n: NonZeroU64) -> Self {
        Self(n)
    }

    /// The counter value.
    #[must_use]
    pub const fn get(self) -> NonZeroU64 {
        self.0
    }
}

/// Every row the ticker can report.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TickKind {
    /// The whole session's uptime.
    Session,
    /// One sub-session tab's uptime.
    SubSession,
    /// One assistant turn, all its steps included.
    Turn,
    /// One parallel tool batch.
    Tools,
    /// One foreground shell.
    Shell,
    /// One background shell.
    Background,
    /// One MCP tool call.
    McpCall,
    /// One subagent run.
    Subagent,
    /// One live MCP server.
    McpServer,
}

impl TickKind {
    /// The kind of a scope's own tick. `None` for `Tool`: a tool has no uptime tick.
    #[must_use]
    pub const fn for_scope(scope: Scope) -> Option<Self> {
        match scope {
            Scope::Session => Some(Self::Session),
            Scope::SubSession(_) => Some(Self::SubSession),
            Scope::Subagent(_) => Some(Self::Subagent),
            Scope::Tool(_) => None,
        }
    }
}

/// One row of `Ticker::snapshot`. `Clone`, not `Copy`: `label` is an `Arc`.
#[derive(Clone, Debug)]
pub struct Tick {
    /// The tick.
    pub id: TickId,
    /// What the tick measures.
    pub kind: TickKind,
    /// Who the tick belongs to.
    pub owner: Scope,
    /// The tool call this tick times, if any: the join key to the tool's row.
    pub tool: Option<ToolId>,
    /// What the row shows: a shell's command, a server's name. Never logged.
    pub label: Option<Arc<str>>,
    /// How long the tick has run.
    pub elapsed: Duration,
    /// How long since the last activity. `None` for a tick with no timer.
    pub quiet: Option<Duration>,
}

/// The kinds that carry a hard and a stale timer. Only these convert into a
/// `TickKind`, so a timer on a `Turn` does not compile.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WatchKind {
    /// One foreground shell.
    Shell,
    /// One background shell.
    Background,
    /// One MCP tool call.
    McpCall,
    /// One subagent run.
    Subagent,
    /// One live MCP server.
    McpServer,
}

impl From<WatchKind> for TickKind {
    fn from(kind: WatchKind) -> Self {
        match kind {
            WatchKind::Shell => Self::Shell,
            WatchKind::Background => Self::Background,
            WatchKind::McpCall => Self::McpCall,
            WatchKind::Subagent => Self::Subagent,
            WatchKind::McpServer => Self::McpServer,
        }
    }
}

/// The two limits of one killable kind, as durations. `check` turns them into deadlines.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Budget {
    /// The hard timer. `None` = no ceiling.
    pub timeout: Option<Duration>,
    /// The stale timer: the longest gap with no activity.
    pub idle_timeout: Duration,
}

/// The budgets for every killable kind, plus the quiet allowed before the stale timer starts.
/// Read each time a timer is armed, so a config change applies to the next timer only.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Timeouts {
    /// Quiet allowed after the start before the stale timer counts. Covers a slow
    /// first output: a compile, a model's first token.
    pub idle_grace: Duration,
    /// A foreground shell.
    pub shell: Budget,
    /// A background shell: no ceiling unless the model sets one.
    pub background: Budget,
    /// One MCP tool call.
    pub mcp_call: Budget,
    /// One subagent run.
    pub subagent: Budget,
    /// A live MCP server: no ceiling.
    pub mcp_server: Budget,
}

impl Default for Timeouts {
    /// Forgiving on purpose: well above an average run. The stale timer does the everyday work.
    fn default() -> Self {
        let secs = Duration::from_secs;
        Self {
            idle_grace: secs(10),
            shell: Budget {
                timeout: Some(secs(1800)),
                idle_timeout: secs(300),
            },
            background: Budget {
                timeout: None,
                idle_timeout: secs(1800),
            },
            mcp_call: Budget {
                timeout: Some(secs(900)),
                idle_timeout: secs(300),
            },
            subagent: Budget {
                timeout: Some(secs(3600)),
                idle_timeout: secs(600),
            },
            mcp_server: Budget {
                timeout: None,
                idle_timeout: secs(1800),
            },
        }
    }
}

/// Why a watch's token fired.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Expiry {
    /// The hard timer ran out.
    Timeout,
    /// No activity for the stale budget.
    Stale,
}

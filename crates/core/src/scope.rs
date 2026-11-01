//! What a call is about: the session, a sub-session tab, a subagent or a tool.
//! One type for the whole harness.

use crate::id::{SubSessionId, SubagentId, ToolId};
use std::fmt;

/// The owner of a call, a record, a tick or a tab. A tick's owner is never `Tool`:
/// the tool rides in its own field.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Scope {
    /// The whole session.
    Session,
    /// One sub-session tab.
    SubSession(SubSessionId),
    /// One subagent.
    Subagent(SubagentId),
    /// One tool call.
    Tool(ToolId),
}

/// `session` or the id, the same spelling in every log line.
impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Session => f.write_str("session"),
            Self::SubSession(id) => id.fmt(f),
            Self::Subagent(id) => id.fmt(f),
            Self::Tool(id) => id.fmt(f),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displays_session_or_the_id() {
        assert_eq!(Scope::Session.to_string(), "session");
        let id: SubSessionId = "ss2m7y6t".parse().unwrap();
        assert_eq!(Scope::SubSession(id).to_string(), "ss2m7y6t");
    }
}

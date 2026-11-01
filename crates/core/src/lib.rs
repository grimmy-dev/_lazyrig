//! The words every lazyrig crate shares: ids, scopes and timing types. No I/O, no async,
//! no traits and no workspace dependency, so it is the one crate every arrow points at.

pub mod id;
pub mod pulse;
pub mod scope;
pub mod time;

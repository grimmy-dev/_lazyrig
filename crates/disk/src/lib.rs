//! File mechanics for lazyrig: where home is, crash-safe writes, the OS trash and the temp sweep.
//!
//! Stateless free functions over sync std. No async and no logging: a caller on the runtime wraps
//! one call in `spawn_blocking`, and logs the errors with its own rules.

mod error;
pub mod paths;
pub mod temp;
pub mod trash;
pub mod write;
pub use error::{Error, Result};

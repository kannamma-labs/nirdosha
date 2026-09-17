//! RFC 0021's project-scoped, revisioned authoring service. Source is v2 only.
pub mod error;
pub mod hash;
pub mod journal;
pub mod mcp;
pub mod schema;
pub mod store;

pub use error::{Error, Result};
pub use store::Graph;
mod workflow;
mod emitter;
pub mod analysis;

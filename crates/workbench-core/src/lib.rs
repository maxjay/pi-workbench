//! Files and persisted workspace state, independent of the renderer and agent.

pub mod baseline;
pub mod diff;
pub mod files;
pub mod git;
pub mod layout;
pub mod merge;
pub mod settings;
pub mod structure;
pub mod tasks;

pub const MAX_OPEN_DOCUMENTS: usize = 64;

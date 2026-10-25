//! Packaging-only orchestration. The installed programs remain independent.
//! Application formats are read by their owning core, not by installer parsers.
#[cfg(windows)]
mod controls;
pub mod discovery;
pub mod execution;
#[cfg(windows)]
mod msi;
pub mod platform;
pub mod transaction;

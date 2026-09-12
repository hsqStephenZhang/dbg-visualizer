//! GDB visualization: opt-in, bounded, recursive Rust formatting.
//! Requires nightly specialization. Deriving does not instrument business operations.
#[cfg(feature = "derive")]
pub use dbgvis_macros::{Visualize, main, register, register_type};
pub use dbgvis_runtime::*;

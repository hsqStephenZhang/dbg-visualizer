//! GDB visualization: opt-in, bounded, recursive Rust formatting.
//! Requires nightly specialization. Deriving does not instrument business operations.
#[cfg(feature = "derive")]
pub use dbgvis_macros::{Visualize, main, visualizers};
pub use visualizer_runtime::*;

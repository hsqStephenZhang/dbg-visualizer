//! Nightly recursive formatting. Only the selected formatter is ever executed.
#![feature(specialization)]
#![allow(incomplete_features)]

// Embeds the GDB bridge (gdb.py wrapped by build.rs) into `.debug_gdb_scripts` of every
// executable that links this crate. Requires debuginfo in the final build and a target that
// emits the section (Linux ELF; Apple and MSVC targets never do).
include!(concat!(env!("OUT_DIR"), "/autoload.rs"));

mod dispatch;
mod formatter;
mod impls;
mod protocol;
mod writer;

pub use formatter::{FormatOptions, Formatter, Group, Outcome, Output, format_into};
#[doc(hidden)]
pub use linkme as __linkme;
pub use protocol::*;
pub use writer::BoundedWriter;
pub type Result = std::fmt::Result;

/// Recursive logical view. No `Any`, `'static`, `Debug` or `Display` supertrait.
/// Propagate formatter errors; arbitrary user code is not sandboxed.
pub trait Visualize {
    fn visualize(&self, out: &mut Formatter<'_>) -> Result;
}

/// Validate at monomorphization time (cargo build, not metadata-only check).
#[doc(hidden)]
pub const fn validate<T: ?Sized>() {
    assert!(
        dispatch::supported::<T>(),
        "dbgvis: no Visualize, Debug or Display implementation"
    );
}

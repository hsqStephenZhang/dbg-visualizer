//! Q0 counterexample: independent blanket bounds do not express trait priority.
use std::fmt::{Debug, Display};

trait Visualize {}
impl<T: Debug> Visualize for T {}
impl<T: Display> Visualize for T {}

fn main() {}

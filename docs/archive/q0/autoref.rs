//! Q0 counterexample: concrete autoref dispatch does not specialize generic bodies.
use std::fmt::{Debug, Display};

trait Visualize {
    fn visualize(&self) -> String;
}

struct Probe<'a, T: ?Sized>(&'a T);
trait Select {
    fn select(self) -> String;
}
impl<T: Visualize + ?Sized> Select for &&&Probe<'_, T> {
    fn select(self) -> String {
        self.0.visualize()
    }
}
impl<T: Debug + ?Sized> Select for &&Probe<'_, T> {
    fn select(self) -> String {
        format!("debug:{:?}", self.0)
    }
}
impl<T: Display + ?Sized> Select for &Probe<'_, T> {
    fn select(self) -> String {
        format!("display:{}", self.0)
    }
}

#[derive(Debug)]
struct Both;
impl Visualize for Both {
    fn visualize(&self) -> String {
        "visualize:Both".into()
    }
}

struct DisplayOnly;
impl Display for DisplayOnly {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DisplayOnly")
    }
}

fn generic_debug<T: Debug>(value: &T) -> String {
    (&&&Probe(value)).select()
}

#[cfg(unbounded)]
fn generic_unbounded<T>(value: &T) -> String {
    (&&&Probe(value)).select()
}

fn main() {
    let concrete = (&&&Probe(&Both)).select();
    let generic = generic_debug(&Both);
    assert_eq!(concrete, "visualize:Both");
    assert_eq!(generic, "debug:Both");
    assert_eq!((&&&Probe(&DisplayOnly)).select(), "display:DisplayOnly");
    println!("CONCRETE={concrete}");
    println!("GENERIC={generic}");
    println!("AUTOREF_GENERIC_PRIORITY_LOST");
}

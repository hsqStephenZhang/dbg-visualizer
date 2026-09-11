//! Separate crate: its generic Visualize implementation is not visible to dispatch's source.
use q0_dispatch::{Visualize, render, text, validate};
use std::collections::HashMap;
use std::fmt::{self, Debug, Display, Write};
use std::hash::{BuildHasherDefault, DefaultHasher};
use std::marker::PhantomData;

struct VisualizeOnly;
impl Visualize for VisualizeOnly {
    fn visualize(&self, out: &mut dyn Write) -> fmt::Result {
        out.write_str("visualize-only")
    }
}
#[derive(Debug)]
struct DebugOnly;
struct DisplayOnly;
impl Display for DisplayOnly {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str("display-only")
    }
}
struct Both;
impl Debug for Both {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str("debug-both")
    }
}
impl Display for Both {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str("display-both")
    }
}
struct All;
impl Visualize for All {
    fn visualize(&self, out: &mut dyn Write) -> fmt::Result {
        out.write_str("visualize-all")
    }
}
impl Debug for All {
    fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
        panic!("lower priority Debug must not be called")
    }
}
impl Display for All {
    fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
        panic!("lower priority Display must not be called")
    }
}

struct NoTraits;
struct Wrapper<'a, T, Policy, const N: usize> {
    value: &'a T,
    marker: PhantomData<Policy>,
}
impl<T, Policy, const N: usize> Visualize for Wrapper<'_, T, Policy, N> {
    const VALID: () = validate::<T>();
    fn visualize(&self, out: &mut dyn Write) -> fmt::Result {
        write!(out, "Wrapper<{N}>(")?;
        render(self.value, out)?;
        out.write_str(")")
    }
}

fn main() {
    assert_eq!(text(&VisualizeOnly), "visualize-only");
    assert_eq!(text(&DebugOnly), "DebugOnly");
    assert_eq!(text(&DisplayOnly), "display-only");
    assert_eq!(text(&Both), "debug-both");
    assert_eq!(text(&All), "visualize-all");
    assert_eq!(text(&vec![VisualizeOnly]), "[visualize-only]");
    assert_eq!(text(&vec![DebugOnly]), "[DebugOnly]");
    assert_eq!(text(&vec![DisplayOnly]), "[display-only]");
    assert_eq!(text(&vec![All]), "[visualize-all]");
    assert_eq!(
        text(&HashMap::from([(1, vec![All])])),
        "{1: [visualize-all]}"
    );
    let mut map = HashMap::<(), DisplayOnly, BuildHasherDefault<DefaultHasher>>::default();
    map.insert((), DisplayOnly);
    assert_eq!(text(&map), "{(): display-only}");
    let owned = String::from("borrowed");
    let borrowed = owned.as_str();
    let wrapper = Wrapper::<_, NoTraits, 13> {
        value: &borrowed,
        marker: PhantomData,
    };
    assert_eq!(text(&vec![wrapper]), "[Wrapper<13>(\"borrowed\")]");
    #[cfg(missing)]
    println!("{}", text(&vec![NoTraits]));
    println!("NIGHTLY_PRIORITY_GENERIC_CROSS_CRATE_OK");
}

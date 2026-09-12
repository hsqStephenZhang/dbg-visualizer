//! Opt-in two-session driver fixture. No registration for any container below.
#[derive(dbgvis::Visualize)]
struct Point {
    x: i32,
}

#[derive(Debug)]
struct DebugOnly {
    count: u32,
}

#[inline(never)]
fn checkpoint(value: *const ()) {
    std::hint::black_box(value);
}

fn main() {
    dbgvis::enable!();
    let map = std::collections::HashMap::from([(7u64, vec![Some(Point { x: 42 })])]);
    let ordered = indexmap::IndexMap::from([(1u32, String::from("one"))]);
    let debug_only = DebugOnly { count: 9 };
    std::hint::black_box(debug_only.count);
    checkpoint(&(&map, &ordered, &debug_only) as *const _ as *const ());
    std::hint::black_box((&map, &ordered, &debug_only));
}

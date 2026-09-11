//! Stable, read-only typed anchor experiment. Null pointer fields are never dereferenced.
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, DefaultHasher};
use std::marker::PhantomData;

#[repr(C)]
pub struct Anchor<T> {
    pub slot: u64,
    pub typed: *const T,
}
// Immutable metadata; typed is null and used only through debugger type information.
unsafe impl<T> Sync for Anchor<T> {}

pub type Map = HashMap<String, Vec<u64>>;
pub type CustomMap = HashMap<(), u32, BuildHasherDefault<DefaultHasher>>;
#[derive(Debug)]
pub struct Borrowed<'a, Policy, const N: usize> {
    value: &'a str,
    marker: PhantomData<Policy>,
}
#[derive(Debug)]
pub struct Zst;

mod left {
    pub struct State(pub u64);
}
mod right {
    pub struct State(pub u64);
}

pub static ANCHOR_MAP: Anchor<Map> = Anchor {
    slot: 0,
    typed: std::ptr::null(),
};
pub static ANCHOR_CUSTOM: Anchor<CustomMap> = Anchor {
    slot: 1,
    typed: std::ptr::null(),
};
pub static ANCHOR_BORROWED: Anchor<Borrowed<'_, Zst, 17>> = Anchor {
    slot: 2,
    typed: std::ptr::null(),
};
pub static ANCHOR_ZST: Anchor<Zst> = Anchor {
    slot: 3,
    typed: std::ptr::null(),
};
pub static ANCHOR_BORROWED_OTHER: Anchor<Borrowed<'_, Zst, 19>> = Anchor {
    slot: 4,
    typed: std::ptr::null(),
};
pub static ANCHOR_LEFT: Anchor<left::State> = Anchor {
    slot: 5,
    typed: std::ptr::null(),
};
pub static ANCHOR_RIGHT: Anchor<right::State> = Anchor {
    slot: 6,
    typed: std::ptr::null(),
};

#[inline(never)]
fn checkpoint() {
    std::hint::black_box(());
}

fn main() {
    std::hint::black_box((
        &ANCHOR_MAP,
        &ANCHOR_CUSTOM,
        &ANCHOR_BORROWED,
        &ANCHOR_ZST,
        &ANCHOR_BORROWED_OTHER,
        &ANCHOR_LEFT,
        &ANCHOR_RIGHT,
    ));
    let owned = String::from("non-static");
    let map = Map::from([(String::from("key"), vec![1, 2])]);
    let mut custom = CustomMap::default();
    custom.insert((), 42);
    let borrowed = Borrowed::<Zst, 17> {
        value: &owned,
        marker: PhantomData,
    };
    let zst = Zst;
    let borrowed_other = Borrowed::<Zst, 19> {
        value: &owned,
        marker: PhantomData,
    };
    let left = left::State(31);
    let right = right::State(37);
    checkpoint();
    std::hint::black_box((
        &map,
        &custom,
        &borrowed,
        &zst,
        borrowed.value,
        &borrowed_other,
        &left,
        &right,
        left.0,
        right.0,
    ));
}

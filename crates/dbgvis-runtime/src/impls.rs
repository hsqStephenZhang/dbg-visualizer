use crate::{Formatter, Result, Visualize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::fmt::Write;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::Arc;

fn sequence<'a, T: ?Sized + 'a>(
    out: &mut Formatter<'_>,
    values: impl IntoIterator<Item = &'a T>,
) -> Result {
    let mut group = out.sequence()?;
    let mut iter = values.into_iter();
    loop {
        if iter.size_hint().1 == Some(0) {
            break;
        }
        group.ready()?;
        let Some(value) = iter.next() else { break };
        group.item(value)?;
    }
    group.finish()
}

impl<T> Visualize for [T] {
    fn visualize(&self, out: &mut Formatter<'_>) -> Result {
        sequence(out, self)
    }
}
impl<T, const N: usize> Visualize for [T; N] {
    fn visualize(&self, out: &mut Formatter<'_>) -> Result {
        sequence(out, self)
    }
}
impl<T> Visualize for Vec<T> {
    fn visualize(&self, out: &mut Formatter<'_>) -> Result {
        sequence(out, self)
    }
}
impl<T> Visualize for VecDeque<T> {
    fn visualize(&self, out: &mut Formatter<'_>) -> Result {
        sequence(out, self)
    }
}
impl<T, S> Visualize for HashSet<T, S> {
    fn visualize(&self, out: &mut Formatter<'_>) -> Result {
        sequence(out, self)
    }
}
impl<T> Visualize for BTreeSet<T> {
    fn visualize(&self, out: &mut Formatter<'_>) -> Result {
        sequence(out, self)
    }
}
macro_rules! map {
    ($map:ident<$($param:ident),+>) => {
        impl<$($param),+> Visualize for $map<$($param),+> {
            fn visualize(&self, out: &mut Formatter<'_>) -> Result {
                let mut group = out.map()?;
                let mut iter = self.iter();
                while iter.len() != 0 {
                    group.ready()?;
                    let (key, value) = iter.next().expect("exact length");
                    group.entry(key, value)?;
                }
                group.finish()
            }
        }
    }
}
map!(HashMap<K, V, S>);
map!(BTreeMap<K, V>);

impl<T> Visualize for Option<T> {
    fn visualize(&self, out: &mut Formatter<'_>) -> Result {
        match self {
            None => out.write_str("None"),
            Some(value) => {
                let mut group = out.tuple("Some")?;
                group.item(value)?;
                group.finish()
            }
        }
    }
}
impl<T, E> Visualize for std::result::Result<T, E> {
    fn visualize(&self, out: &mut Formatter<'_>) -> Result {
        match self {
            Ok(value) => {
                let mut group = out.tuple("Ok")?;
                group.item(value)?;
                group.finish()
            }
            Err(value) => {
                let mut group = out.tuple("Err")?;
                group.item(value)?;
                group.finish()
            }
        }
    }
}
macro_rules! reference {
    ($($wrapper:ty),+) => { $(impl<T: ?Sized> Visualize for $wrapper {
        fn visualize(&self, out: &mut Formatter<'_>) -> Result { out.value(&**self) }
    })+ }
}
reference!(&T, &mut T, Box<T>, Rc<T>, Arc<T>);
impl<T: ?Sized> Visualize for PhantomData<T> {
    fn visualize(&self, out: &mut Formatter<'_>) -> Result {
        out.write_str("PhantomData")
    }
}
macro_rules! tuple {
    ($($t:ident:$index:tt),+) => {
        impl<$($t),+> Visualize for ($($t,)+) {
            fn visualize(&self, out: &mut Formatter<'_>) -> Result {
                let mut group = out.tuple("")?;
                $(group.item(&self.$index)?;)+
                group.finish()
            }
        }
    }
}
tuple!(A:0);
tuple!(A:0, B:1);
tuple!(A:0, B:1, C:2);
tuple!(A:0, B:1, C:2, D:3);
tuple!(A:0, B:1, C:2, D:3, E:4);
tuple!(A:0, B:1, C:2, D:3, E:4, F:5);

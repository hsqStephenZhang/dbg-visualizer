//! Isolated nightly experiment, NOT the public runtime or a default workspace dependency.
//! Tests late trait selection without unsafe casts, TypeId tricks or 'static bounds.
#![feature(specialization)]
#![allow(incomplete_features)]

use std::collections::HashMap;
use std::fmt::{self, Debug, Display, Write};

pub trait Visualize {
    const VALID: () = ();
    fn visualize(&self, out: &mut dyn Write) -> fmt::Result;
}

trait DisplayDispatch {
    const VALID: ();
    fn render_display(&self, out: &mut dyn Write) -> fmt::Result;
}
impl<T: ?Sized> DisplayDispatch for T {
    default const VALID: () = panic!("dbgvis: no Visualize, Debug or Display implementation");
    default fn render_display(&self, _: &mut dyn Write) -> fmt::Result {
        Err(fmt::Error)
    }
}
impl<T: Display + ?Sized> DisplayDispatch for T {
    const VALID: () = ();
    fn render_display(&self, out: &mut dyn Write) -> fmt::Result {
        write!(out, "{self}")
    }
}

trait DebugDispatch {
    const VALID: ();
    fn render_debug(&self, out: &mut dyn Write) -> fmt::Result;
}
impl<T: ?Sized> DebugDispatch for T {
    default const VALID: () = <T as DisplayDispatch>::VALID;
    default fn render_debug(&self, out: &mut dyn Write) -> fmt::Result {
        self.render_display(out)
    }
}
impl<T: Debug + ?Sized> DebugDispatch for T {
    const VALID: () = ();
    fn render_debug(&self, out: &mut dyn Write) -> fmt::Result {
        write!(out, "{self:?}")
    }
}

trait VisualizeDispatch {
    const VALID: ();
    fn render_visualize(&self, out: &mut dyn Write) -> fmt::Result;
}
impl<T: ?Sized> VisualizeDispatch for T {
    default const VALID: () = <T as DebugDispatch>::VALID;
    default fn render_visualize(&self, out: &mut dyn Write) -> fmt::Result {
        self.render_debug(out)
    }
}
impl<T: Visualize + ?Sized> VisualizeDispatch for T {
    const VALID: () = <T as Visualize>::VALID;
    fn render_visualize(&self, out: &mut dyn Write) -> fmt::Result {
        self.visualize(out)
    }
}

pub const fn validate<T: ?Sized>() {
    <T as VisualizeDispatch>::VALID
}

pub fn render<T: ?Sized>(value: &T, out: &mut dyn Write) -> fmt::Result {
    const { validate::<T>() };
    value.render_visualize(out)
}

pub fn text<T: ?Sized>(value: &T) -> String {
    let mut out = String::new();
    render(value, &mut out).unwrap();
    out
}

impl<T> Visualize for Vec<T> {
    const VALID: () = validate::<T>();
    fn visualize(&self, out: &mut dyn Write) -> fmt::Result {
        out.write_str("[")?;
        for (index, value) in self.iter().enumerate() {
            if index != 0 {
                out.write_str(", ")?;
            }
            render(value, out)?;
        }
        out.write_str("]")
    }
}

impl<K, V, S> Visualize for HashMap<K, V, S> {
    const VALID: () = {
        validate::<K>();
        validate::<V>();
    };
    fn visualize(&self, out: &mut dyn Write) -> fmt::Result {
        out.write_str("{")?;
        for (index, (key, value)) in self.iter().enumerate() {
            if index != 0 {
                out.write_str(", ")?;
            }
            render(key, out)?;
            out.write_str(": ")?;
            render(value, out)?;
        }
        out.write_str("}")
    }
}

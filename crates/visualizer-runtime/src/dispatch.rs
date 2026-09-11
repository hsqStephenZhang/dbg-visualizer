use crate::{Formatter, Result, Visualize};
use std::fmt::{Debug, Display};

trait DisplayDispatch {
    const SUPPORTED: bool;
    fn display(&self, out: &mut Formatter<'_>) -> Result;
}
impl<T: ?Sized> DisplayDispatch for T {
    default const SUPPORTED: bool = false;
    default fn display(&self, _: &mut Formatter<'_>) -> Result {
        Err(std::fmt::Error)
    }
}
impl<T: Display + ?Sized> DisplayDispatch for T {
    const SUPPORTED: bool = true;
    fn display(&self, out: &mut Formatter<'_>) -> Result {
        out.display_leaf(self)
    }
}

trait DebugDispatch {
    const SUPPORTED: bool;
    fn debug(&self, out: &mut Formatter<'_>) -> Result;
}
impl<T: ?Sized> DebugDispatch for T {
    default const SUPPORTED: bool = <T as DisplayDispatch>::SUPPORTED;
    default fn debug(&self, out: &mut Formatter<'_>) -> Result {
        self.display(out)
    }
}
impl<T: Debug + ?Sized> DebugDispatch for T {
    const SUPPORTED: bool = true;
    fn debug(&self, out: &mut Formatter<'_>) -> Result {
        out.debug_leaf(self)
    }
}

trait VisualizeDispatch {
    const SUPPORTED: bool;
    fn render(&self, out: &mut Formatter<'_>) -> Result;
}
impl<T: ?Sized> VisualizeDispatch for T {
    default const SUPPORTED: bool = <T as DebugDispatch>::SUPPORTED;
    default fn render(&self, out: &mut Formatter<'_>) -> Result {
        self.debug(out)
    }
}
impl<T: Visualize + ?Sized> VisualizeDispatch for T {
    const SUPPORTED: bool = true;
    fn render(&self, out: &mut Formatter<'_>) -> Result {
        self.visualize(out)
    }
}

pub const fn supported<T: ?Sized>() -> bool {
    <T as VisualizeDispatch>::SUPPORTED
}
pub fn render<T: ?Sized>(value: &T, out: &mut Formatter<'_>) -> Result {
    const { crate::validate::<T>() };
    value.render(out)
}

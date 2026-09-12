use crate::{BoundedWriter, Result, Visualize, dispatch};
use std::fmt::{self, Debug, Display, Write};

pub const MAX_DEPTH: usize = 128;

#[derive(Clone, Copy, Debug)]
pub struct FormatOptions {
    pub alternate: bool,
    pub max_depth: usize,
    pub max_nodes: usize,
}
impl Default for FormatOptions {
    fn default() -> Self {
        Self {
            alternate: false,
            max_depth: 32,
            max_nodes: 4096,
        }
    }
}

#[repr(u64)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Complete = 0,
    Bytes = 1,
    Depth = 2,
    Nodes = 3,
    Cycle = 4,
    FormatError = 5,
}

pub struct Output<'a> {
    pub text: &'a str,
    pub outcome: Outcome,
    pub nodes: usize,
}

#[derive(Clone, Copy)]
struct Visit {
    address: usize,
    size: usize,
    name: &'static str,
}
const EMPTY: Visit = Visit {
    address: 0,
    size: 0,
    name: "",
};

pub struct Formatter<'a> {
    writer: BoundedWriter<'a>,
    options: FormatOptions,
    path: [Visit; MAX_DEPTH],
    depth: usize,
    nodes: usize,
    outcome: Outcome,
}

impl<'a> Formatter<'a> {
    pub fn new(buffer: &'a mut [u8], options: FormatOptions) -> Self {
        Self {
            writer: BoundedWriter::new(buffer),
            options,
            path: [EMPTY; MAX_DEPTH],
            depth: 0,
            nodes: 0,
            outcome: Outcome::Complete,
        }
    }
    fn stop(&mut self, reason: Outcome) -> Result {
        if self.outcome == Outcome::Complete {
            self.outcome = reason;
        }
        self.writer.write_str("…")?;
        Err(fmt::Error)
    }
    pub fn ready(&mut self) -> Result {
        if self.outcome != Outcome::Complete || self.writer.truncated() {
            return Err(fmt::Error);
        }
        if self.nodes >= self.options.max_nodes {
            return self.stop(Outcome::Nodes);
        }
        if self.depth >= self.options.max_depth.min(MAX_DEPTH) {
            return self.stop(Outcome::Depth);
        }
        if self.writer.remaining() == 0 {
            return self.writer.write_str("…");
        }
        Ok(())
    }
    fn node<T: ?Sized>(
        &mut self,
        value: &T,
        render: impl FnOnce(&T, &mut Self) -> Result,
    ) -> Result {
        self.ready()?;
        let visit = Visit {
            address: std::ptr::from_ref(value).cast::<()>() as usize,
            size: size_of_val(value),
            name: std::any::type_name::<T>(),
        };
        if visit.size != 0
            && self.path[..self.depth]
                .iter()
                .any(|v| v.address == visit.address && v.size == visit.size && v.name == visit.name)
        {
            return self.stop(Outcome::Cycle);
        }
        self.path[self.depth] = visit;
        self.depth += 1;
        self.nodes += 1;
        let result = render(value, self);
        self.depth -= 1;
        result
    }
    pub fn value<T: ?Sized>(&mut self, value: &T) -> Result {
        self.node(value, dispatch::render::<T>)
    }
    pub fn debug<T: Debug + ?Sized>(&mut self, value: &T) -> Result {
        self.node(value, |v, out| out.debug_leaf(v))
    }
    pub fn display<T: Display + ?Sized>(&mut self, value: &T) -> Result {
        self.node(value, |v, out| out.display_leaf(v))
    }
    pub fn visual<T: Visualize + ?Sized>(&mut self, value: &T) -> Result {
        self.node(value, |v, out| v.visualize(out))
    }
    pub(crate) fn debug_leaf<T: Debug + ?Sized>(&mut self, value: &T) -> Result {
        if self.options.alternate {
            write!(self, "{value:#?}")
        } else {
            write!(self, "{value:?}")
        }
    }
    pub(crate) fn display_leaf<T: Display + ?Sized>(&mut self, value: &T) -> Result {
        if self.options.alternate {
            write!(self, "{value:#}")
        } else {
            write!(self, "{value}")
        }
    }
    pub fn record(&mut self, name: &str) -> std::result::Result<Group<'_, 'a>, fmt::Error> {
        self.write_str(name)?;
        self.write_str(" {")?;
        Ok(Group {
            out: self,
            count: 0,
            close: "}",
            spaced: true,
        })
    }
    pub fn tuple(&mut self, name: &str) -> std::result::Result<Group<'_, 'a>, fmt::Error> {
        self.write_str(name)?;
        self.write_str("(")?;
        Ok(Group {
            out: self,
            count: 0,
            close: ")",
            spaced: false,
        })
    }
    pub fn sequence(&mut self) -> std::result::Result<Group<'_, 'a>, fmt::Error> {
        self.write_str("[")?;
        Ok(Group {
            out: self,
            count: 0,
            close: "]",
            spaced: false,
        })
    }
    pub fn map(&mut self) -> std::result::Result<Group<'_, 'a>, fmt::Error> {
        self.write_str("{")?;
        Ok(Group {
            out: self,
            count: 0,
            close: "}",
            spaced: false,
        })
    }
    fn indent(&mut self, depth: usize) -> Result {
        self.write_str("\n")?;
        for _ in 0..depth {
            self.write_str("  ")?;
        }
        Ok(())
    }
    pub fn outcome(&self, result: Result) -> Outcome {
        if self.writer.truncated() {
            Outcome::Bytes
        } else if self.outcome != Outcome::Complete {
            self.outcome
        } else if result.is_err() {
            Outcome::FormatError
        } else {
            Outcome::Complete
        }
    }
    pub fn written(&self) -> usize {
        self.writer.len()
    }
    pub fn nodes(&self) -> usize {
        self.nodes
    }
}
impl Write for Formatter<'_> {
    fn write_str(&mut self, value: &str) -> Result {
        if self.outcome != Outcome::Complete {
            return Err(fmt::Error);
        }
        self.writer.write_str(value)
    }
}

pub struct Group<'f, 'b> {
    out: &'f mut Formatter<'b>,
    count: usize,
    close: &'static str,
    spaced: bool,
}
impl Group<'_, '_> {
    pub fn ready(&mut self) -> Result {
        self.out.ready()
    }
    fn prefix(&mut self, name: Option<&str>) -> Result {
        self.out.ready()?;
        if self.count != 0 {
            self.out.write_str(",")?;
        }
        if self.out.options.alternate {
            self.out.indent(self.out.depth)?;
        } else if self.count != 0 || self.spaced {
            self.out.write_str(" ")?;
        }
        if let Some(name) = name {
            self.out.write_str(name)?;
            self.out.write_str(": ")?;
        }
        self.count += 1;
        Ok(())
    }
    pub fn field<T: ?Sized>(&mut self, name: &str, value: &T) -> Result {
        self.prefix(Some(name))?;
        self.out.value(value)
    }
    pub fn field_debug<T: Debug + ?Sized>(&mut self, name: &str, value: &T) -> Result {
        self.prefix(Some(name))?;
        self.out.debug(value)
    }
    pub fn field_display<T: Display + ?Sized>(&mut self, name: &str, value: &T) -> Result {
        self.prefix(Some(name))?;
        self.out.display(value)
    }
    pub fn item<T: ?Sized>(&mut self, value: &T) -> Result {
        self.prefix(None)?;
        self.out.value(value)
    }
    pub fn item_debug<T: Debug + ?Sized>(&mut self, value: &T) -> Result {
        self.prefix(None)?;
        self.out.debug(value)
    }
    pub fn item_display<T: Display + ?Sized>(&mut self, value: &T) -> Result {
        self.prefix(None)?;
        self.out.display(value)
    }
    pub fn entry<K: ?Sized, V: ?Sized>(&mut self, key: &K, value: &V) -> Result {
        self.prefix(None)?;
        self.out.value(key)?;
        self.out.write_str(": ")?;
        self.out.value(value)
    }
    pub fn finish(self) -> Result {
        if self.count != 0 {
            if self.out.options.alternate {
                self.out.indent(self.out.depth.saturating_sub(1))?;
            } else if self.spaced {
                self.out.write_str(" ")?;
            }
        }
        self.out.write_str(self.close)
    }
}

pub fn format_into<'a, T: ?Sized>(
    value: &T,
    buffer: &'a mut [u8],
    options: FormatOptions,
) -> Output<'a> {
    let mut formatter = Formatter::new(buffer, options);
    let result = formatter.value(value);
    let outcome = formatter.outcome(result);
    let written = formatter.written();
    let nodes = formatter.nodes();
    Output {
        text: std::str::from_utf8(&buffer[..written]).expect("bounded writer preserves UTF-8"),
        outcome,
        nodes,
    }
}

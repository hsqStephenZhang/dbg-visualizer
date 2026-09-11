//! Deliberately failing formatters, always run in disposable debugger subprocesses.
use std::fmt;

struct Fault {
    kind: u32,
}

impl fmt::Display for Fault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            1 => panic!("controlled fmt panic"),
            2 => Err(fmt::Error),
            3 => loop {
                std::hint::spin_loop();
            },
            4 => std::process::exit(23),
            _ => formatter.write_str("healthy"),
        }
    }
}

#[dbgvis::visualizers]
mod visualizers {
    #[dbgvis(display)]
    type Fault = super::Fault;
}

#[inline(never)]
fn checkpoint() {
    std::hint::black_box(0);
}

fn main() {
    dbgvis::enable!(visualizers);
    let mode = std::env::args().nth(1).unwrap_or_default();
    let fault = Fault {
        kind: match mode.as_str() {
            "panic" => 1,
            "error" => 2,
            "hang" => 3,
            "exit" => 4,
            _ => 0,
        },
    };
    checkpoint();
    std::hint::black_box(&fault);
}

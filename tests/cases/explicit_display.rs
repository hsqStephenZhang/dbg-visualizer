#[derive(Debug)] struct DebugOnly;
#[derive(dv::Visualize)] struct Root { #[dbgvis(via = "display")] value: DebugOnly }
fn main() {}

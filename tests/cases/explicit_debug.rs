struct NoDebug;
#[derive(dv::Visualize)] struct Root { #[dbgvis(via = "debug")] value: NoDebug }
fn main() {}

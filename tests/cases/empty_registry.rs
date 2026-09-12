#[derive(dv::Visualize)] struct Generic<T>(T);
#[derive(dv::Visualize)] struct Borrowed<'a>(&'a str);
#[derive(dv::Visualize)] #[dbgvis(no_register)] struct Nested { x: u32 }
#[dv::main] fn main() { assert!(dv::VIS_TYPES.is_empty()); }

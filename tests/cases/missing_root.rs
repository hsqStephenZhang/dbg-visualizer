#[derive(Debug)] struct DebugOnly;
dv::register_type!(DebugOnly; display);
#[dv::main] fn main() {}

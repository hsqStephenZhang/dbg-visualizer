struct Missing;
#[derive(dv::Visualize)] struct Root<T> { value: T }
dv::register_type!(Root<Missing>);
#[dv::main] fn main() {}

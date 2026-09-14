struct Missing;
#[derive(dv::Visualize)] struct Root<T> { value: T }
dv::register_type!(Root<Missing>);
#[dv::main] fn main() {
    let mut buffer = [0; 128];
    let text = dv::format_into(&Root { value: Missing }, &mut buffer, Default::default()).text;
    assert!(text.starts_with("Root { value: <unformattable ") && text.ends_with("::Missing> }"), "{text}");
}

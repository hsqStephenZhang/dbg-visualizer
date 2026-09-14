struct Missing;
#[derive(dv::Visualize)] struct Root { value: Missing }
#[dv::main] fn main() {
    let mut buffer = [0; 128];
    let text = dv::format_into(&Root { value: Missing }, &mut buffer, Default::default()).text;
    assert!(text.starts_with("Root { value: <unformattable ") && text.ends_with("::Missing> }"), "{text}");
    assert_eq!(dv::VIS_TYPES.len(), 1);
}

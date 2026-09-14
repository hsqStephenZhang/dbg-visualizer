struct Missing;
dv::register_type!(std::collections::HashMap<u8, Vec<Missing>>);
#[dv::main] fn main() {
    let mut buffer = [0; 128];
    let map = std::collections::HashMap::from([(1u8, vec![Missing])]);
    let text = dv::format_into(&map, &mut buffer, Default::default()).text;
    assert!(text.starts_with("{1: [<unformattable ") && text.ends_with("::Missing>]}"), "{text}");
}

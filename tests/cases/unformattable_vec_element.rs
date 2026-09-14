struct Missing;
fn main() {
    let mut buffer = [0; 128];
    let items = vec![Missing];
    let text = dv::format_into(&items, &mut buffer, Default::default()).text;
    assert!(text.starts_with("[<unformattable ") && text.ends_with("::Missing>]"), "{text}");
}

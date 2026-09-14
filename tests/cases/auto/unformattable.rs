struct NoFormat;
fn main() {
    dv::enable!();
    let missing = vec![NoFormat];
    let mut buffer = [0; 128];
    let text = dv::format_into(&missing, &mut buffer, Default::default()).text;
    assert!(text.starts_with("[<unformattable ") && text.ends_with("::NoFormat>]"), "{text}");
    std::hint::black_box(&missing);
    println!("PLACEHOLDER_EXECUTED");
}

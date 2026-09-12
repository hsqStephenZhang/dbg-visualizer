struct NoFormat;
fn main() {
    dv::enable!();
    let missing: Vec<NoFormat> = Vec::new();
    std::hint::black_box(missing);
}

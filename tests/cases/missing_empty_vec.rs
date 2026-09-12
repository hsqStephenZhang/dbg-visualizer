struct Missing;
fn main() {
    let mut buffer = [0; 64];
    let empty: Vec<Missing> = Vec::new();
    std::hint::black_box(dv::format_into(&empty, &mut buffer, Default::default()));
}

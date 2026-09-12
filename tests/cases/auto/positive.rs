use dv::Visualize;
#[derive(dv::Visualize)] struct Point { x: u32 }
#[dv::register(display)] struct Explicit;
impl std::fmt::Display for Explicit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("explicit") }
}
struct NoFormat;
#[derive(Debug)] struct Borrowed<'a>(&'a str);
#[derive(Debug)] struct Packet<T, const N: usize> { values: [T; N] }
mod hidden {
    #[derive(Debug)] struct Secret;
    #[inline(never)] pub fn work() { let secret = Secret; std::hint::black_box(secret); }
}
#[inline(never)] fn generic<T>(value: T) {
    let values = vec![value];
    std::hint::black_box(values);
}
fn main() {
    dv::enable!();
    let point = Point { x: 1 };
    let explicit = Explicit;
    let missing = NoFormat;
    let borrowed = Borrowed("text");
    #[derive(Debug)] struct Local;
    let local = Local;
    let map = std::collections::HashMap::from([(1u64, vec![Some(Point { x: 3 })])]);
    let ordered = idx::IndexMap::from([(1u32, "one".to_string())]);
    let packet = Packet { values: [2u8; 17] };
    generic(9u16);
    hidden::work();
    std::hint::black_box((point, explicit, missing, borrowed.0, local, map, ordered, packet.values));
    println!("CONTRACT_EXECUTED");
}

mod a { dv::register_type!(u32); }
mod b { #[dv::register(display)] type Root = u32; }
fn main() { dv::enable!(); assert_eq!(dv::collected_roots().len(), 1); }

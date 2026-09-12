mod a { #[derive(dv::Visualize)] pub struct Same(u32); }
mod b { #[derive(dv::Visualize)] pub struct Same(u32); }
#[dv::main] fn main() {
    #[derive(dv::Visualize)] struct Local(u32);
    assert_eq!(dv::VIS_TYPES.len(), 3);
}

#[derive(dv::Visualize)] struct Automatic { x: u32 }
#[derive(dv::Visualize)] #[dbgvis(no_register)] struct Nested { x: u32 }
#[derive(dv::Visualize)] struct Generic<T>(T);
#[dv::register] #[derive(Debug)] struct DebugOnly(u32);
#[derive(Debug)] struct PlainDebug;
#[dv::register(display)] struct DisplayOnly;
impl std::fmt::Display for DisplayOnly {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("display") }
}
dv::register_type!(Generic<Nested>);
#[dv::main] fn main() {
    assert_eq!(dv::VIS_TYPES.len(), 4);
    let mut buf = [0; 128];
    assert_eq!(dv::format_into(&Generic(Nested { x: 3 }), &mut buf, Default::default()).text,
        "Generic(Nested { x: 3 })");
}

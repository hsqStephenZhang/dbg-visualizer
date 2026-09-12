fn main() {
    { #[derive(dv::Visualize)] struct Same(u32); }
    { #[derive(dv::Visualize)] struct Same(f32); }
    let error = std::panic::catch_unwind(dv::collected_roots).err().expect("collision accepted");
    let message = error.downcast_ref::<String>().map(String::as_str)
        .or_else(|| error.downcast_ref::<&str>().copied()).unwrap();
    assert!(message.contains("different concrete type identities"), "{message}");
}

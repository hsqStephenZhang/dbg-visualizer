fn main() {
    assert_eq!(std::any::type_name::<old::Shared>(), std::any::type_name::<new::Shared>());
    assert_eq!(size_of::<old::Shared>(), size_of::<new::Shared>());
    assert_eq!(align_of::<old::Shared>(), align_of::<new::Shared>());
    assert_ne!(std::any::TypeId::of::<old::Shared>(), std::any::TypeId::of::<new::Shared>());
    assert_eq!(dv::VIS_TYPES.len(), 2);
    let error = std::panic::catch_unwind(|| { dv::enable!(); }).err().expect("collision accepted");
    let message = error.downcast_ref::<String>().map(String::as_str)
        .or_else(|| error.downcast_ref::<&str>().copied()).unwrap();
    assert!(message.contains("different concrete type identities"), "{message}");
}

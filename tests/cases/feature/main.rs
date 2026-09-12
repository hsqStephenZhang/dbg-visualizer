struct NoTraits;
#[cfg_attr(feature = "visualize", derive(dbgvis::Visualize))]
struct App {
    a: demo_a::Record<demo_b::Item, NoTraits>,
    c: demo_c::State,
    label: demo_c::Label,
    #[cfg_attr(feature = "visualize", dbgvis(skip))]
    skipped: NoTraits,
}
fn main() {
    #[cfg(feature = "visualize")]
    dbgvis::enable!();
    let app = App { a: demo_a::Record::new(demo_b::Item::new(8)),
        c: demo_c::State::Pending, label: demo_c::Label, skipped: NoTraits };
    #[cfg(feature = "visualize")]
    {
        let mut buffer = [0; 512];
        let text = dbgvis::format_into(&app, &mut buffer, Default::default()).text;
        assert!(text.contains("count: 8") && text.contains("State::Pending"));
        assert!(!text.contains("skipped"));
        assert!(text.contains("label: Display-only"));
        assert_eq!(dbgvis::VIS_TYPES.len(), 4); // App, Item, UnusedRegistered, State
    }
    std::hint::black_box((&app.a, &app.c, &app.label, &app.skipped));
}

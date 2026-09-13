fn main() {
    dv::enable!();
    // Outside types are still eligible when used by the executable itself.
    let used = scan_reexports::ForeignUsed;
    std::hint::black_box(&used);
    println!("REEXPORTS_EXECUTED");
}

mod a { #[dv::register(debug)] type Root<'a> = &'a str; }
mod b { #[dv::register(display)] type Root<'b> = &'b str; }
fn main() { dv::enable!(); assert_eq!(dv::collected_roots().len(), 1); }

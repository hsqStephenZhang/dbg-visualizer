struct Borrowed<'a>(&'a str);
impl std::fmt::Display for Borrowed<'static> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(self.0) }
}
#[dv::register(display)] type Root<'a> = Borrowed<'a>;
#[dv::main] fn main() {}

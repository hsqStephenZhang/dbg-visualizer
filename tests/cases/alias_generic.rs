use std::{fmt, marker::PhantomData, collections::HashMap};
struct Policy;
struct Label;
impl fmt::Display for Label { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str("label") } }
#[derive(dv::Visualize)]
struct Wrapper<'a, T, P, const N: usize> { value: T, text: &'a str, policy: PhantomData<P> }
mod first {
    #[dv::register]
    type Root<'a> = super::Wrapper<'a, std::collections::HashMap<u8, Vec<super::Label>>, super::Policy, 9>;
    #[cfg(any())] #[dv::register] type Disabled = Missing;
}
mod second { dv::register_type!(u32; display); }
fn main() {
    dv::enable!();
    assert_eq!(dv::VIS_TYPES.len(), 2);
    let text = String::from("borrowed");
    let value = Wrapper::<_, Policy, 9> { value: HashMap::from([(1, vec![Label])]), text: &text, policy: PhantomData };
    let mut buffer = [0; 256];
    assert_eq!(dv::format_into(&value, &mut buffer, Default::default()).text,
        "Wrapper { value: {1: [label]}, text: \"borrowed\", policy: PhantomData }");
}

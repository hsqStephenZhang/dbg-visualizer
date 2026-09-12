#[cfg_attr(feature = "visualize", derive(dbgvis::Visualize))]
pub struct Record<T, Policy> { value: T, policy: std::marker::PhantomData<Policy> }
impl<T, P> Record<T, P> {
    pub fn new(value: T) -> Self { Self { value, policy: std::marker::PhantomData } }
}

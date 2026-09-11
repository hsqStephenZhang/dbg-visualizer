use std::marker::PhantomData;

#[cfg_attr(feature = "visualize", derive(dbgvis::Visualize))]
pub struct Record<T, Policy> {
    value: T,
    policy: PhantomData<Policy>,
}
impl<T, Policy> Record<T, Policy> {
    pub fn new(value: T) -> Self {
        Self {
            value,
            policy: PhantomData,
        }
    }
    pub fn value(&self) -> &T {
        &self.value
    }
}

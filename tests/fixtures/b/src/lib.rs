#[cfg_attr(feature = "visualize", derive(dbgvis::Visualize))]
pub struct Item {
    count: u32,
}
impl Item {
    pub fn new(count: u32) -> Self {
        Self { count }
    }
    pub fn count(&self) -> u32 {
        self.count
    }
}

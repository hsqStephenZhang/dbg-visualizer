#[cfg_attr(feature = "visualize", derive(dbgvis::Visualize))]
pub struct Item { count: u32 }
impl Item { pub fn new(count: u32) -> Self { Self { count } } }
#[cfg_attr(feature = "visualize", derive(dbgvis::Visualize))]
pub struct UnusedRegistered { pub value: u32 }

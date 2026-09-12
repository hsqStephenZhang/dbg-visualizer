#[cfg_attr(feature = "visualize", derive(dbgvis::Visualize))]
pub enum State { Pending }
pub struct Label;
impl std::fmt::Display for Label {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Display-only")
    }
}

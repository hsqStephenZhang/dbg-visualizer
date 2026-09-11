#[cfg_attr(feature = "visualize", derive(dbgvis::Visualize))]
pub enum State {
    Ready { message: String },
    Pending,
}

/// Third-party-style private representation, deliberately Display-only (no Visualize).
pub struct Label(String);
impl Label {
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }
}
impl std::fmt::Display for Label {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

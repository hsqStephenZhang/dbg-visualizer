pub use foreign::*;

// Re-exports within a selected crate must remain traversable.
mod selected {
    #[derive(Debug)]
    pub struct SelectedFunctionLocal;
    #[derive(Debug)]
    pub struct SelectedMethodLocal;
    pub struct Selected;

    pub fn worker() {
        let value = SelectedFunctionLocal;
        std::hint::black_box(&value);
    }

    impl Selected {
        pub fn method() {
            let value = SelectedMethodLocal;
            std::hint::black_box(&value);
        }
    }
}

pub use selected::*;

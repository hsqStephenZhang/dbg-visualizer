// Included as an unused constant expression, solely for Cargo's dependency tracking.
concat!(
    include_str!("driver.rs"),
    include_str!("wrapper.py"),
    include_str!("build.py"),
    env!("DBGVIS_AUTO_CRATE"),
)

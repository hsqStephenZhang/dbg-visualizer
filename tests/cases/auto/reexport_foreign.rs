#[derive(Debug)]
pub struct ForeignUsed;
#[derive(Debug)]
pub struct ForeignFunctionLocal;
#[derive(Debug)]
pub struct ForeignStructLocal;
#[derive(Debug)]
pub struct ForeignEnumLocal;
#[derive(Debug)]
pub struct ForeignUnionLocal;

// Inline exports retain MIR even though this dependency is outside the workspace.
// None of these functions is called by the executable.
#[inline]
pub fn foreign_worker() {
    let value = ForeignFunctionLocal;
    std::hint::black_box(&value);
}

pub struct ForeignStruct;
impl ForeignStruct {
    #[inline]
    pub fn method() {
        let value = ForeignStructLocal;
        std::hint::black_box(&value);
    }
}

pub enum ForeignEnum {
    Variant,
}
impl ForeignEnum {
    #[inline]
    pub fn method() {
        let value = ForeignEnumLocal;
        std::hint::black_box(&value);
    }
}

pub union ForeignUnion {
    pub value: u8,
}
impl ForeignUnion {
    #[inline]
    pub fn method() {
        let value = ForeignUnionLocal;
        std::hint::black_box(&value);
    }
}

pub mod foreign_module {
    #[derive(Debug)]
    pub struct ForeignModuleLocal;

    #[inline]
    pub fn worker() {
        let value = ForeignModuleLocal;
        std::hint::black_box(&value);
    }
}

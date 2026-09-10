use crate::{Entry, Runtime};

pub const ABI_VERSION: u64 = 1;
pub const MAGIC: [u8; 8] = *b"DBGVIS01";
pub const MAX_PAGE: usize = 64;
pub const MAX_CHILDREN: usize = MAX_PAGE * 2;
pub const OUTPUT_CAPACITY: usize = capacity(option_env!("DBGVIS_BUFFER_BYTES"));

const fn capacity(value: Option<&str>) -> usize {
    let Some(text) = value else { return 65536 };
    let bytes = text.as_bytes();
    let mut n = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        assert!(
            bytes[i] >= b'0' && bytes[i] <= b'9',
            "DBGVIS_BUFFER_BYTES must be decimal"
        );
        n = n * 10 + (bytes[i] - b'0') as usize;
        assert!(n <= 16 * 1024 * 1024, "DBGVIS_BUFFER_BYTES exceeds 16 MiB");
        i += 1;
    }
    assert!(n > 0, "DBGVIS_BUFFER_BYTES must be positive");
    n
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok = 0,
    Truncated = 1,
    Unsupported = 2,
    InvalidRequest = 3,
    AbiMismatch = 4,
    Busy = 5,
    FormatError = 6,
    Panic = 7,
}

pub const FORMAT: u64 = 1;
pub const CHILD_COUNT: u64 = 2;
pub const CHILD_PAGE: u64 = 3;
pub const DEBUG: u64 = 1;
pub const DISPLAY: u64 = 2;
pub const CHILDREN: u64 = 4;
pub const STRUCT: u64 = 0;
pub const SEQUENCE: u64 = 1;
pub const MAP: u64 = 2;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Request {
    pub abi: u64,
    pub size: u64,
    pub sequence: u64,
    pub operation: u64,
    pub type_id: u64,
    pub object: u64,
    pub mode: u64,
    pub alternate: u64,
    pub capacity: u64,
    pub start: u64,
    pub count: u64,
}

impl Request {
    pub const EMPTY: Self = Self {
        abi: ABI_VERSION,
        size: size_of::<Self>() as u64,
        sequence: 0,
        operation: FORMAT,
        type_id: 0,
        object: 0,
        mode: DEBUG,
        alternate: 0,
        capacity: 4096,
        start: 0,
        count: 0,
    };
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Response {
    pub sequence: u64,
    pub status: u64,
    pub written: u64,
    pub total: u64,
    pub returned: u64,
    pub shape: u64,
}

impl Response {
    pub const EMPTY: Self = Self {
        sequence: 0,
        status: 0,
        written: 0,
        total: 0,
        returned: 0,
        shape: 0,
    };
}

/// Child names and type names are UTF-8, zero-terminated and never truncated.
/// kind=0 is an addressable typed value; kind=1 is an inline unsigned scalar;
/// kind=2 additionally supplies the bytes of a borrowed str (the address is the &str slot).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Child {
    pub name: [u8; 64],
    pub type_name: [u8; 192],
    pub address: u64,
    pub size: u64,
    pub align: u64,
    pub kind: u64,
    pub data: u64,
    pub length: u64,
}

impl Child {
    pub const EMPTY: Self = Self {
        name: [0; 64],
        type_name: [0; 192],
        address: 0,
        size: 0,
        align: 0,
        kind: 0,
        data: 0,
        length: 0,
    };

    /// The returned descriptor must not outlive this debugger stop or the borrowed value.
    pub fn borrowed<T>(name: &str, value: &T) -> Result<Self, Status> {
        let mut child = Self::EMPTY;
        copy_text(&mut child.name, name)?;
        copy_text(&mut child.type_name, std::any::type_name::<T>())?;
        child.address = std::ptr::from_ref(value) as usize as u64;
        child.size = size_of::<T>() as u64;
        child.align = align_of::<T>() as u64;
        Ok(child)
    }

    pub fn string(name: &str, value: &&str) -> Result<Self, Status> {
        let mut child = Self::borrowed(name, value)?;
        child.kind = 2;
        child.data = value.as_ptr() as usize as u64;
        child.length = value.len() as u64;
        Ok(child)
    }

    /// Copy a computed scalar into the response instead of returning a temporary's address.
    pub fn scalar(name: &str, value: u64) -> Result<Self, Status> {
        let mut child = Self::EMPTY;
        copy_text(&mut child.name, name)?;
        copy_text(&mut child.type_name, "u64")?;
        child.size = 8;
        child.align = align_of::<u64>() as u64;
        child.kind = 1;
        child.data = value;
        Ok(child)
    }
}

fn copy_text<const N: usize>(out: &mut [u8; N], value: &str) -> Result<(), Status> {
    if value.len() >= N || value.contains('\0') {
        return Err(Status::Unsupported);
    }
    out[..value.len()].copy_from_slice(value.as_bytes());
    Ok(())
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Text {
    pub pointer: *const u8,
    pub length: usize,
}

impl Text {
    pub const fn new(text: &'static str) -> Self {
        Self {
            pointer: text.as_ptr(),
            length: text.len(),
        }
    }
}

// Text only refers to immutable static string literals.
unsafe impl Sync for Text {}

/// Read-only discovery header. v1 supports 64-bit little-endian Linux targets.
#[repr(C)]
pub struct Module {
    pub magic: [u8; 8],
    pub abi: u32,
    pub size: u32,
    pub pointer_width: u32,
    pub little_endian: u32,
    pub entry_size: u32,
    pub request_size: u32,
    pub response_size: u32,
    pub child_size: u32,
    pub entries: *const Entry,
    pub entry_count: usize,
    pub request: *mut Request,
    pub response: *mut Response,
    pub output: *mut u8,
    pub output_capacity: usize,
    pub children: *mut Child,
    pub children_capacity: usize,
    pub dispatcher: unsafe extern "C" fn(usize) -> u32,
}

// Pointees are immutable entries or Runtime memory protected by its dispatcher.
unsafe impl Sync for Module {}

impl Module {
    pub const fn new<const N: usize>(
        runtime: &'static Runtime<N>,
        entries: &'static [Entry],
        dispatcher: unsafe extern "C" fn(usize) -> u32,
    ) -> Self {
        Self {
            magic: MAGIC,
            abi: ABI_VERSION as u32,
            size: size_of::<Self>() as u32,
            pointer_width: size_of::<usize>() as u32,
            little_endian: cfg!(target_endian = "little") as u32,
            entry_size: size_of::<Entry>() as u32,
            request_size: size_of::<Request>() as u32,
            response_size: size_of::<Response>() as u32,
            child_size: size_of::<Child>() as u32,
            entries: entries.as_ptr(),
            entry_count: entries.len(),
            request: runtime.request.get(),
            response: runtime.response.get(),
            output: runtime.output.get().cast(),
            output_capacity: N,
            children: runtime.children.get().cast(),
            children_capacity: MAX_CHILDREN,
            dispatcher,
        }
    }
}

//! A debugger-only, versioned bridge to explicitly registered formatting implementations.
//! Call [`retain`] from reachable application code to preserve the bridge under LTO.
//!
//! ```compile_fail
//! struct NoDisplay;
//! visualizer_runtime::register_visualizers! {
//!     NoDisplay => { name: "NoDisplay", display }
//! }
//! ```

mod protocol;
mod writer;
pub use protocol::*;
use std::cell::UnsafeCell;
use std::fmt::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
pub use writer::BoundedWriter;

pub type FormatFn = unsafe fn(usize, &mut BoundedWriter<'_>, bool) -> fmt::Result;
pub type CountFn = unsafe fn(usize) -> usize;
pub type PageFn = unsafe fn(usize, usize, usize, &mut [Child]) -> Result<usize, Status>;

/// Implementations must return only live, correctly typed child addresses, never temporaries.
///
/// # Safety
/// `page` descriptors may be used to construct references in subsequent debugger calls.
/// The adapter must preserve the actual type, size, alignment and borrowed lifetime.
pub unsafe trait Children<T> {
    const SHAPE: u64;
    fn count(value: &T) -> usize;
    fn page(value: &T, start: usize, count: usize, out: &mut [Child]) -> Result<usize, Status>;
}

#[repr(C)]
pub struct Entry {
    pub name: Text,
    pub gdb_alias: Text,
    pub lldb_alias: Text,
    pub size: usize,
    pub align: usize,
    pub capabilities: u64,
    pub shape: u64,
    pub debug_fn: Option<FormatFn>,
    pub display_fn: Option<FormatFn>,
    pub count_fn: Option<CountFn>,
    pub page_fn: Option<PageFn>,
}

impl Entry {
    pub const fn new<T>(name: &'static str) -> Self {
        Self {
            name: Text::new(name),
            gdb_alias: Text::new(""),
            lldb_alias: Text::new(""),
            size: size_of::<T>(),
            align: align_of::<T>(),
            capabilities: 0,
            shape: STRUCT,
            debug_fn: None,
            display_fn: None,
            count_fn: None,
            page_fn: None,
        }
    }
    pub const fn aliases(mut self, gdb: &'static str, lldb: &'static str) -> Self {
        self.gdb_alias = Text::new(gdb);
        self.lldb_alias = Text::new(lldb);
        self
    }
    pub const fn debug<T: fmt::Debug>(mut self) -> Self {
        self.debug_fn = Some(format_debug::<T>);
        self.capabilities |= DEBUG;
        self
    }
    pub const fn display<T: fmt::Display>(mut self) -> Self {
        self.display_fn = Some(format_display::<T>);
        self.capabilities |= DISPLAY;
        self
    }
    pub const fn children<T, A: Children<T>>(mut self) -> Self {
        self.count_fn = Some(count::<T, A>);
        self.page_fn = Some(page::<T, A>);
        self.shape = A::SHAPE;
        self.capabilities |= CHILDREN;
        self
    }
}

unsafe fn format_debug<T: fmt::Debug>(
    address: usize,
    writer: &mut BoundedWriter<'_>,
    alternate: bool,
) -> fmt::Result {
    // SAFETY: the dispatcher caller guarantees a live T at address for this call only.
    let value = unsafe { &*(address as *const T) };
    if alternate {
        write!(writer, "{value:#?}")
    } else {
        write!(writer, "{value:?}")
    }
}
unsafe fn format_display<T: fmt::Display>(
    address: usize,
    writer: &mut BoundedWriter<'_>,
    alternate: bool,
) -> fmt::Result {
    let value = unsafe { &*(address as *const T) };
    if alternate {
        write!(writer, "{value:#}")
    } else {
        write!(writer, "{value}")
    }
}
unsafe fn count<T, A: Children<T>>(address: usize) -> usize {
    A::count(unsafe { &*(address as *const T) })
}
unsafe fn page<T, A: Children<T>>(
    address: usize,
    start: usize,
    count: usize,
    out: &mut [Child],
) -> Result<usize, Status> {
    A::page(unsafe { &*(address as *const T) }, start, count, out)
}

pub struct Runtime<const N: usize> {
    request: UnsafeCell<Request>,
    response: UnsafeCell<Response>,
    output: UnsafeCell<[u8; N]>,
    children: UnsafeCell<[Child; MAX_CHILDREN]>,
    busy: AtomicBool,
}

// Rust access to all cells is serialized by busy. The debugger must serialize mailbox writes
// while the process is stopped and never write during an active dispatcher invocation.
unsafe impl<const N: usize> Sync for Runtime<N> {}

impl<const N: usize> Default for Runtime<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Runtime<N> {
    pub const fn new() -> Self {
        Self {
            request: UnsafeCell::new(Request::EMPTY),
            response: UnsafeCell::new(Response::EMPTY),
            output: UnsafeCell::new([0; N]),
            children: UnsafeCell::new([Child::EMPTY; MAX_CHILDREN]),
            busy: AtomicBool::new(false),
        }
    }

    /// # Safety
    /// The request's object must designate a live, aligned instance of its exact registered
    /// type, with invariants valid for shared access. No thread or debugger may mutate the
    /// mailbox or object during this call. Name/size checks do not establish these guarantees.
    pub unsafe fn dispatch(&self, request_address: usize, entries: &[Entry]) -> Status {
        if request_address != self.request.get() as usize {
            return Status::InvalidRequest;
        }
        if self
            .busy
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return Status::Busy;
        }
        struct Guard<'a>(&'a AtomicBool);
        impl Drop for Guard<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        let _guard = Guard(&self.busy);
        let request = unsafe { *self.request.get() };
        let mut response = Response {
            sequence: request.sequence,
            ..Response::EMPTY
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
            self.execute(&request, &mut response, entries)
        }));
        let status = match result {
            Ok(status) => status,
            Err(payload) => {
                // A user panic payload may itself have a panicking destructor. Catch that too;
                // only a secondary panicking payload must be leaked to keep the C boundary intact.
                if let Err(secondary) =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(payload)))
                {
                    std::mem::forget(secondary);
                }
                Status::Panic
            }
        };
        if !matches!(status, Status::Ok | Status::Truncated) {
            response.written = 0;
            response.returned = 0;
        }
        response.status = status as u64;
        unsafe {
            *self.response.get() = response;
        }
        status
    }

    unsafe fn execute(&self, r: &Request, response: &mut Response, entries: &[Entry]) -> Status {
        if r.abi != ABI_VERSION || r.size != size_of::<Request>() as u64 {
            return Status::AbiMismatch;
        }
        let Some(entry) = usize::try_from(r.type_id)
            .ok()
            .and_then(|id| entries.get(id))
        else {
            return Status::InvalidRequest;
        };
        let Ok(address) = usize::try_from(r.object) else {
            return Status::InvalidRequest;
        };
        if address == 0
            || !address.is_multiple_of(entry.align)
            || address.checked_add(entry.size).is_none()
        {
            return Status::InvalidRequest;
        }
        response.shape = entry.shape;
        match r.operation {
            FORMAT => {
                if r.capacity == 0 || r.capacity > N as u64 || r.alternate > 1 {
                    return Status::InvalidRequest;
                }
                let function = match r.mode {
                    DEBUG => entry.debug_fn,
                    DISPLAY => entry.display_fn,
                    _ => return Status::InvalidRequest,
                };
                let Some(function) = function else {
                    return Status::Unsupported;
                };
                let output = unsafe { &mut *self.output.get() };
                let mut writer = BoundedWriter::new(&mut output[..r.capacity as usize]);
                let result = unsafe { function(address, &mut writer, r.alternate != 0) };
                response.written = writer.len() as u64;
                if writer.truncated() {
                    Status::Truncated
                } else if result.is_err() {
                    Status::FormatError
                } else {
                    Status::Ok
                }
            }
            CHILD_COUNT | CHILD_PAGE => {
                let (Some(count_fn), Some(page_fn)) = (entry.count_fn, entry.page_fn) else {
                    return Status::Unsupported;
                };
                if r.operation == CHILD_PAGE
                    && (r.count == 0
                        || r.count > MAX_PAGE as u64
                        || r.start.checked_add(r.count).is_none())
                {
                    return Status::InvalidRequest;
                }
                let total = unsafe { count_fn(address) };
                response.total = total as u64;
                if r.operation == CHILD_COUNT {
                    return Status::Ok;
                }
                if r.start > total as u64 {
                    return Status::InvalidRequest;
                }
                let start = r.start as usize;
                let count = (r.count as usize).min(total - start);
                let output = unsafe { &mut *self.children.get() };
                let multiplier = if entry.shape == MAP { 2 } else { 1 };
                match unsafe { page_fn(address, start, count, &mut output[..count * multiplier]) } {
                    Ok(written) if written == count * multiplier => {
                        response.returned = written as u64;
                        Status::Ok
                    }
                    Ok(_) => Status::InvalidRequest,
                    Err(status) => status,
                }
            }
            _ => Status::InvalidRequest,
        }
    }
}

/// Make the exported descriptor and dispatcher reachable even with section GC and LTO.
#[inline(never)]
pub fn retain(module: &'static Module) {
    std::hint::black_box(module);
}

#[macro_export]
macro_rules! register_visualizers {
    ($($ty:ty => { name: $name:literal $(, $($options:tt)*)? }),+ $(,)?) => {
        static DBG_VIS_ENTRIES: &[$crate::Entry] = &[
            $($crate::register_visualizers!(@options $ty, $crate::Entry::new::<$ty>($name); $($($options)*)?)),+
        ];
        static DBG_VIS_RUNTIME: $crate::Runtime<{ $crate::OUTPUT_CAPACITY }> = $crate::Runtime::new();
        #[unsafe(no_mangle)]
        pub static DBG_VIS_MODULE_V1: $crate::Module = $crate::Module::new(&DBG_VIS_RUNTIME, DBG_VIS_ENTRIES, dbgvis_dispatch_v1);
        /// # Safety
        /// The mailbox must refer to a live object of the exact registered type, in a valid
        /// shared-access state. The debugger must serialize calls and mailbox writes.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn dbgvis_dispatch_v1(request: usize) -> u32 {
            unsafe { DBG_VIS_RUNTIME.dispatch(request, DBG_VIS_ENTRIES) as u32 }
        }
    };
    (@options $ty:ty, $entry:expr;) => { $entry };
    (@options $ty:ty, $entry:expr; debug $(, $($rest:tt)*)?) => {
        $crate::register_visualizers!(@options $ty, $entry.debug::<$ty>(); $($($rest)*)?)
    };
    (@options $ty:ty, $entry:expr; display $(, $($rest:tt)*)?) => {
        $crate::register_visualizers!(@options $ty, $entry.display::<$ty>(); $($($rest)*)?)
    };
    (@options $ty:ty, $entry:expr; children: $adapter:ty $(, $($rest:tt)*)?) => {
        $crate::register_visualizers!(@options $ty, $entry.children::<$ty, $adapter>(); $($($rest)*)?)
    };
    (@options $ty:ty, $entry:expr; aliases: [$gdb:literal, $lldb:literal] $(, $($rest:tt)*)?) => {
        $crate::register_visualizers!(@options $ty, $entry.aliases($gdb, $lldb); $($($rest)*)?)
    };
}

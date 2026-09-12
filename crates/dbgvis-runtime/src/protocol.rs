use crate::{FormatOptions, Formatter, Outcome, Visualize};
use std::any::TypeId;
use std::cell::UnsafeCell;
use std::fmt::{Debug, Display};
use std::marker::PhantomData;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering};

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;

pub const ABI: u64 = 2;
pub const AUTO: u64 = 1;
pub const VISUALIZE: u64 = 2;
pub const DEBUG: u64 = 4;
pub const DISPLAY: u64 = 8;
pub const OUTPUT_CAPACITY: usize = capacity(option_env!("DBGVIS_BUFFER_BYTES"));
const fn capacity(value: Option<&str>) -> usize {
    let Some(text) = value else { return 65536 };
    let mut n = 0usize;
    let mut index = 0;
    while index < text.len() {
        let digit = text.as_bytes()[index];
        assert!(
            digit >= b'0' && digit <= b'9',
            "DBGVIS_BUFFER_BYTES must be decimal"
        );
        n = n * 10 + (digit - b'0') as usize;
        assert!(n <= 16 * 1024 * 1024, "DBGVIS_BUFFER_BYTES exceeds 16 MiB");
        index += 1;
    }
    assert!(n != 0, "DBGVIS_BUFFER_BYTES must be positive");
    n
}

#[repr(u64)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ok = 0,
    Limited = 1,
    Unsupported = 2,
    Invalid = 3,
    AbiMismatch = 4,
    Busy = 5,
    FormatError = 6,
    Panic = 7,
    NotReady = 8,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Text {
    pointer: *const u8,
    len: usize,
}
impl Text {
    fn new(text: &'static str) -> Self {
        Self {
            pointer: text.as_ptr(),
            len: text.len(),
        }
    }

    fn as_str(self) -> &'static str {
        // Text values are constructed only from &'static str metadata.
        unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(self.pointer, self.len)) }
    }
}
// Points only to immutable static strings.
unsafe impl Sync for Text {}
unsafe impl Send for Text {}

type FormatFn = unsafe fn(usize, &mut Formatter<'_>) -> crate::Result;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Entry {
    name: Text,
    anchor: Text,
    size: usize,
    align: usize,
    capabilities: u64,
    default_mode: u64,
    auto_fn: Option<FormatFn>,
    visual_fn: Option<FormatFn>,
    debug_fn: Option<FormatFn>,
    display_fn: Option<FormatFn>,
}
#[derive(Clone, Copy)]
pub struct Root {
    key: TypeId,
    identity: TypeId,
    name: &'static str,
    entry: Entry,
}

/// Linker-collected metadata factories. Factories never format business values.
#[linkme::distributed_slice]
pub static VIS_TYPES: [fn() -> Root];

#[doc(hidden)]
pub fn collected_roots() -> Vec<Root> {
    assert!(VIS_TYPES.len() <= 4096, "dbgvis: invalid root count");
    let mut roots: Vec<_> = VIS_TYPES.iter().map(|describe| describe()).collect();
    coalesce_roots(&mut roots);
    roots
}

/// Merge registrations for one concrete type emitted by multiple crates.  A
/// dependency may register (for example) `u64` and the final binary may also
/// discover/register it.  The formatter functions are interchangeable only
/// when their lifetime-erased concrete TypeIds agree. Names and layout alone
/// never prove this: different dependency versions can share both.
fn coalesce_roots(roots: &mut Vec<Root>) {
    // Link order is unspecified.  Sort the anchor as a deterministic tie-break
    // before choosing the retained metadata and default mode.
    roots.sort_by_key(|root| (root.name, root.entry.anchor.as_str()));
    let mut markers = std::collections::HashMap::new();
    for root in roots.iter() {
        if let Some(identity) = markers.insert(root.key, root.identity) {
            assert_eq!(
                identity, root.identity,
                "dbgvis: duplicate root registration marker"
            );
        }
    }
    let mut merged: Vec<Root> = Vec::with_capacity(roots.len());
    for root in roots.drain(..) {
        if let Some(previous) = merged.last_mut()
            && previous.name == root.name
        {
            assert_eq!(
                previous.identity, root.identity,
                "dbgvis: same type name has different concrete type identities"
            );
            assert_eq!(
                (previous.entry.size, previous.entry.align),
                (root.entry.size, root.entry.align),
                "dbgvis: same type name has incompatible size/alignment"
            );
            previous.entry.capabilities |= root.entry.capabilities;
            previous.entry.auto_fn = previous.entry.auto_fn.or(root.entry.auto_fn);
            previous.entry.visual_fn = previous.entry.visual_fn.or(root.entry.visual_fn);
            previous.entry.debug_fn = previous.entry.debug_fn.or(root.entry.debug_fn);
            previous.entry.display_fn = previous.entry.display_fn.or(root.entry.display_fn);
            continue;
        }
        merged.push(root);
    }
    *roots = merged;
}
pub struct Registration<T> {
    root: Root,
    invariant: PhantomData<fn(T) -> T>,
}
impl<T> Registration<T> {
    pub fn new<Marker: 'static>(anchor: &'static str) -> Self
    where
        T: 'static,
    {
        // SAFETY: the identity type is exactly T.
        unsafe { Self::new_lifetime_erased::<T, Marker>(anchor) }
    }

    /// Construct a registration for a lifetime-generic formatter.
    ///
    /// # Safety
    /// Identity must be exactly T with its free lifetimes replaced by 'static;
    /// no type/const arguments or nominal type identity may change. This identity
    /// permits merging function pointers from other registrations. It does not
    /// extend the lifetime of any value. Prefer the registration macros.
    #[doc(hidden)]
    pub unsafe fn new_lifetime_erased<Identity: 'static, Marker: 'static>(
        anchor: &'static str,
    ) -> Self {
        Self {
            root: Root {
                key: TypeId::of::<Marker>(),
                identity: TypeId::of::<Identity>(),
                name: std::any::type_name::<T>(),
                entry: Entry {
                    name: Text::new(std::any::type_name::<T>()),
                    anchor: Text::new(anchor),
                    size: size_of::<T>(),
                    align: align_of::<T>(),
                    capabilities: 0,
                    default_mode: 0,
                    auto_fn: None,
                    visual_fn: None,
                    debug_fn: None,
                    display_fn: None,
                },
            },
            invariant: PhantomData,
        }
    }
    fn capability(&mut self, mode: u64) {
        self.root.entry.capabilities |= mode;
        if self.root.entry.default_mode == 0 {
            self.root.entry.default_mode = mode;
        }
    }
    pub fn auto(mut self) -> Self {
        const { crate::validate::<T>() };
        self.capability(AUTO);
        self.root.entry.auto_fn = Some(auto::<T>);
        self
    }
    pub fn visualize(mut self) -> Self
    where
        T: Visualize,
    {
        self.capability(VISUALIZE);
        self.root.entry.visual_fn = Some(visual::<T>);
        self
    }
    pub fn debug(mut self) -> Self
    where
        T: Debug,
    {
        self.capability(DEBUG);
        self.root.entry.debug_fn = Some(debug::<T>);
        self
    }
    pub fn display(mut self) -> Self
    where
        T: Display,
    {
        self.capability(DISPLAY);
        self.root.entry.display_fn = Some(display::<T>);
        self
    }
    pub fn default_mode(mut self, mode: u64) -> Self {
        self.root.entry.default_mode = mode;
        self
    }
    pub fn finish(self) -> Root {
        assert!(
            self.root.entry.capabilities & self.root.entry.default_mode != 0,
            "dbgvis: default mode is not registered"
        );
        self.root
    }
}
unsafe fn auto<T>(address: usize, out: &mut Formatter<'_>) -> crate::Result {
    out.value(unsafe { &*(address as *const T) })
}
unsafe fn visual<T: Visualize>(address: usize, out: &mut Formatter<'_>) -> crate::Result {
    out.visual(unsafe { &*(address as *const T) })
}
unsafe fn debug<T: Debug>(address: usize, out: &mut Formatter<'_>) -> crate::Result {
    out.debug(unsafe { &*(address as *const T) })
}
unsafe fn display<T: Display>(address: usize, out: &mut Formatter<'_>) -> crate::Result {
    out.display(unsafe { &*(address as *const T) })
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Request {
    pub abi: u64,
    pub size: u64,
    pub sequence: u64,
    pub slot: u64,
    pub object: u64,
    pub mode: u64,
    pub capacity: u64,
    pub alternate: u64,
    pub max_depth: u64,
    pub max_nodes: u64,
}
impl Request {
    const EMPTY: Self = Self {
        abi: ABI,
        size: size_of::<Self>() as u64,
        sequence: 0,
        slot: 0,
        object: 0,
        mode: 0,
        capacity: 4096,
        alternate: 0,
        max_depth: 32,
        max_nodes: 4096,
    };
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Response {
    pub sequence: u64,
    pub status: u64,
    pub written: u64,
    pub outcome: u64,
    pub nodes: u64,
}
impl Response {
    const EMPTY: Self = Self {
        sequence: 0,
        status: Status::NotReady as u64,
        written: 0,
        outcome: 0,
        nodes: 0,
    };
}

pub struct Runtime {
    request: UnsafeCell<Request>,
    response: UnsafeCell<Response>,
    output: UnsafeCell<[u8; OUTPUT_CAPACITY]>,
    busy: AtomicBool,
    entries: OnceLock<Box<[Entry]>>,
}
// Access to mailboxes is serialized by the debugger and busy; reads while stopped only.
unsafe impl Sync for Runtime {}
impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}
impl Runtime {
    pub const fn new() -> Self {
        Self {
            request: UnsafeCell::new(Request::EMPTY),
            response: UnsafeCell::new(Response::EMPTY),
            output: UnsafeCell::new([0; OUTPUT_CAPACITY]),
            busy: AtomicBool::new(false),
            entries: OnceLock::new(),
        }
    }
    pub fn enable(&'static self, module: &'static Module, roots: impl FnOnce() -> Vec<Root>) {
        let entries = self.entries.get_or_init(|| {
            let roots = roots();
            assert!(roots.len() <= 4096, "dbgvis: invalid root count");
            let mut roots = roots;
            coalesce_roots(&mut roots);
            roots
                .into_iter()
                .map(|r| r.entry)
                .collect::<Vec<_>>()
                .into_boxed_slice()
        });
        module
            .entries
            .store(entries.as_ptr().cast_mut(), Ordering::Relaxed);
        module.count.store(entries.len(), Ordering::Relaxed);
        module.ready.store(1, Ordering::Release);
        std::hint::black_box(module);
    }
    /// # Safety
    /// Only the published mailbox address is accepted. Its object must be a live exact
    /// registered T, aligned, valid for shared access, with all target threads stopped.
    /// Debugger mailbox writes and dispatch must be serialized; metadata does not prove liveness.
    pub unsafe fn dispatch(&self, address: usize) -> u64 {
        if address != self.request.get() as usize {
            return Status::Invalid as u64;
        }
        if self
            .busy
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return Status::Busy as u64;
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
            self.execute(request, &mut response)
        }));
        let status = match result {
            Ok(status) => status,
            Err(payload) => {
                if let Err(second) =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(payload)))
                {
                    std::mem::forget(second);
                }
                Status::Panic
            }
        };
        if status != Status::Ok && status != Status::Limited {
            response.written = 0;
        }
        response.status = status as u64;
        unsafe {
            *self.response.get() = response;
        }
        status as u64
    }
    unsafe fn execute(&self, request: Request, response: &mut Response) -> Status {
        if request.abi != ABI || request.size != size_of::<Request>() as u64 {
            return Status::AbiMismatch;
        }
        let Some(entries) = self.entries.get() else {
            return Status::NotReady;
        };
        let Some(entry) = entries.get(request.slot as usize) else {
            return Status::Invalid;
        };
        if request.object == 0
            || !(request.object as usize).is_multiple_of(entry.align)
            || (request.object as usize).checked_add(entry.size).is_none()
            || request.capacity == 0
            || request.capacity > OUTPUT_CAPACITY as u64
            || request.alternate > 1
            || request.max_depth == 0
            || request.max_depth > 128
            || request.max_nodes == 0
            || request.max_nodes > 1_000_000
        {
            return Status::Invalid;
        }
        let mode = if request.mode == 0 {
            entry.default_mode
        } else {
            request.mode
        };
        let function = match mode {
            AUTO => entry.auto_fn,
            VISUALIZE => entry.visual_fn,
            DEBUG => entry.debug_fn,
            DISPLAY => entry.display_fn,
            _ => None,
        };
        let Some(function) = function else {
            return Status::Unsupported;
        };
        let buffer = unsafe { &mut *self.output.get() };
        let mut out = Formatter::new(
            &mut buffer[..request.capacity as usize],
            FormatOptions {
                alternate: request.alternate == 1,
                max_depth: request.max_depth as usize,
                max_nodes: request.max_nodes as usize,
            },
        );
        let result = unsafe { function(request.object as usize, &mut out) };
        let outcome = out.outcome(result);
        response.written = out.written() as u64;
        response.outcome = outcome as u64;
        response.nodes = out.nodes() as u64;
        match outcome {
            Outcome::Complete => Status::Ok,
            Outcome::FormatError => Status::FormatError,
            _ => Status::Limited,
        }
    }
}

#[repr(C)]
pub struct Module {
    magic: [u8; 8],
    abi: u64,
    size: u64,
    pointer_width: u64,
    endian: u64,
    entry_size: u64,
    request_size: u64,
    response_size: u64,
    ready: AtomicUsize,
    entries: AtomicPtr<Entry>,
    count: AtomicUsize,
    request: *mut Request,
    response: *mut Response,
    output: *mut u8,
    capacity: u64,
    dispatcher: unsafe extern "C" fn(usize) -> u64,
}
// Immutable pointers to this runtime; published atomics are initialized only by enable.
unsafe impl Sync for Module {}
impl Module {
    pub const fn new(
        runtime: &'static Runtime,
        dispatcher: unsafe extern "C" fn(usize) -> u64,
    ) -> Self {
        Self {
            magic: *b"DBGVIS02",
            abi: ABI,
            size: size_of::<Self>() as u64,
            pointer_width: size_of::<usize>() as u64,
            endian: if cfg!(target_endian = "little") { 1 } else { 2 },
            entry_size: size_of::<Entry>() as u64,
            request_size: size_of::<Request>() as u64,
            response_size: size_of::<Response>() as u64,
            ready: AtomicUsize::new(0),
            entries: AtomicPtr::new(std::ptr::null_mut()),
            count: AtomicUsize::new(0),
            request: runtime.request.get(),
            response: runtime.response.get(),
            output: runtime.output.get().cast(),
            capacity: OUTPUT_CAPACITY as u64,
            dispatcher,
        }
    }
}

#[macro_export]
macro_rules! enable {
    () => {{
        static __DBG_RUNTIME: $crate::Runtime = $crate::Runtime::new();
        #[unsafe(no_mangle)]
        static DBG_VIS_MODULE_V2: $crate::Module =
            $crate::Module::new(&__DBG_RUNTIME, dbgvis_dispatch_v2);
        /// # Safety
        /// Requires a valid exact registered object and serialized stopped-process access.
        #[unsafe(no_mangle)]
        unsafe extern "C" fn dbgvis_dispatch_v2(request: usize) -> u64 {
            unsafe { __DBG_RUNTIME.dispatch(request) }
        }
        __DBG_RUNTIME.enable(&DBG_VIS_MODULE_V2, $crate::collected_roots);
    }};
}

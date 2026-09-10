use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::fmt;
use visualizer_runtime::*;

struct CountingAllocator;
thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.try_with(Cell::get).unwrap_or(false) {
            let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
        }
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if COUNTING.try_with(Cell::get).unwrap_or(false) {
            let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
        }
        unsafe { System.realloc(pointer, layout, size) }
    }
}
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

struct TextValue<'a>(&'a str);
impl fmt::Display for TextValue<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if formatter.alternate() {
            formatter.write_str("alt:")?;
        }
        formatter.write_str(self.0)
    }
}
impl fmt::Debug for TextValue<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("debug:")?;
        formatter.write_str(self.0)
    }
}

struct Failure(bool);
impl fmt::Display for Failure {
    fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0 {
            panic!("controlled format panic");
        }
        Err(fmt::Error)
    }
}

struct Reentry<'a>(&'a Runtime<32>);
impl fmt::Display for Reentry<'_> {
    fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Nested dispatch uses the same valid mailbox, but must return BUSY before reading it.
        let address = REENTRY_ADDRESS.with(Cell::get);
        assert_eq!(unsafe { self.0.dispatch(address, &[]) }, Status::Busy);
        Ok(())
    }
}
thread_local! { static REENTRY_ADDRESS: Cell<usize> = const { Cell::new(0) }; }

static ENTRIES: &[Entry] = &[
    Entry::new::<TextValue>("TextValue")
        .debug::<TextValue>()
        .display::<TextValue>(),
    Entry::new::<Failure>("Failure").display::<Failure>(),
    Entry::new::<Reentry>("Reentry").display::<Reentry>(),
];
unsafe extern "C" fn unused(_: usize) -> u32 {
    0
}

struct Harness {
    runtime: &'static Runtime<32>,
    module: Module,
}
impl Harness {
    fn new() -> Self {
        let runtime = Box::leak(Box::new(Runtime::<32>::new()));
        Self {
            runtime,
            module: Module::new(runtime, ENTRIES, unused),
        }
    }
    fn request<T>(&self, value: &T, type_id: u64) -> Request {
        Request {
            object: std::ptr::from_ref(value) as usize as u64,
            type_id,
            capacity: 32,
            mode: DISPLAY,
            sequence: 91,
            ..Request::EMPTY
        }
    }
    fn submit(&self, request: Request) -> (Status, Response, Vec<u8>) {
        unsafe {
            self.module.request.write(request);
        }
        let status = unsafe { self.runtime.dispatch(self.module.request as usize, ENTRIES) };
        let response = unsafe { self.module.response.read() };
        let bytes =
            unsafe { std::slice::from_raw_parts(self.module.output, response.written as usize) }
                .to_vec();
        assert_eq!(response.status, status as u64);
        assert_eq!(response.sequence, request.sequence);
        (status, response, bytes)
    }
}

#[test]
fn wire_layout_is_stable_on_v1_target() {
    assert_eq!(size_of::<usize>(), 8);
    assert_eq!(size_of::<Module>(), 112);
    assert_eq!(size_of::<Entry>(), 112);
    assert_eq!(size_of::<Request>(), 88);
    assert_eq!(size_of::<Response>(), 48);
    assert_eq!(size_of::<Child>(), 304);
    assert_eq!(std::mem::offset_of!(Module, entries), 40);
    assert_eq!(std::mem::offset_of!(Entry, debug_fn), 80);
}

#[test]
fn scalar_copies_value_and_metadata_never_truncates() {
    let child = Child::scalar("computed", u64::MAX).unwrap();
    assert_eq!(child.address, 0);
    assert_eq!(child.kind, 1);
    assert_eq!(child.data, u64::MAX);
    assert!(matches!(
        Child::scalar(&"x".repeat(64), 1),
        Err(Status::Unsupported)
    ));
    assert!(matches!(
        Child::scalar("bad\0name", 1),
        Err(Status::Unsupported)
    ));
}

#[test]
fn formats_borrowed_data_and_distinguishes_modes() {
    let harness = Harness::new();
    let owned = String::from("中\0文");
    let value = TextValue(&owned);
    let mut request = harness.request(&value, 0);
    assert_eq!(harness.submit(request).2, owned.as_bytes());
    request.mode = DEBUG;
    assert_eq!(harness.submit(request).2, "debug:中\0文".as_bytes());
    request.mode = DISPLAY;
    request.alternate = 1;
    assert_eq!(harness.submit(request).2, "alt:中\0文".as_bytes());
    request.alternate = 0;
    request.capacity = 2;
    let (status, response, bytes) = harness.submit(request);
    assert_eq!(status, Status::Truncated);
    assert_eq!(response.written, 0);
    assert!(bytes.is_empty());
    request.capacity = 3;
    assert_eq!(harness.submit(request).2, "中".as_bytes());
}

#[test]
fn invalid_requests_are_rejected_before_formatting() {
    let harness = Harness::new();
    let value = TextValue("test");
    let request = harness.request(&value, 0);
    for (mutated, expected) in [
        (Request { abi: 99, ..request }, Status::AbiMismatch),
        (Request { size: 1, ..request }, Status::AbiMismatch),
        (
            Request {
                type_id: 99,
                ..request
            },
            Status::InvalidRequest,
        ),
        (
            Request {
                object: 0,
                ..request
            },
            Status::InvalidRequest,
        ),
        (
            Request {
                capacity: 33,
                ..request
            },
            Status::InvalidRequest,
        ),
        (
            Request {
                capacity: 0,
                ..request
            },
            Status::InvalidRequest,
        ),
        (
            Request {
                operation: 99,
                ..request
            },
            Status::InvalidRequest,
        ),
        (
            Request {
                alternate: 2,
                ..request
            },
            Status::InvalidRequest,
        ),
        (
            Request {
                operation: CHILD_COUNT,
                ..request
            },
            Status::Unsupported,
        ),
    ] {
        assert_eq!(harness.submit(mutated).0, expected);
    }
    assert_eq!(
        unsafe { harness.runtime.dispatch(0, ENTRIES) },
        Status::InvalidRequest
    );
}

#[test]
fn errors_panic_and_reentry_do_not_escape_dispatcher() {
    let harness = Harness::new();
    for (value, status) in [
        (Failure(false), Status::FormatError),
        (Failure(true), Status::Panic),
    ] {
        assert_eq!(harness.submit(harness.request(&value, 1)).0, status);
    }
    let value = Reentry(harness.runtime);
    REENTRY_ADDRESS.with(|address| address.set(harness.module.request as usize));
    assert_eq!(harness.submit(harness.request(&value, 2)).0, Status::Ok);
    let value = TextValue("still usable");
    assert_eq!(
        harness.submit(harness.request(&value, 0)).2,
        b"still usable"
    );
}

#[test]
fn bridge_does_not_allocate_for_nonallocating_formatter() {
    let harness = Harness::new();
    let value = TextValue("constant");
    unsafe {
        harness.module.request.write(harness.request(&value, 0));
    }
    ALLOCATIONS.with(|count| count.set(0));
    COUNTING.with(|flag| flag.set(true));
    let status = unsafe {
        harness
            .runtime
            .dispatch(harness.module.request as usize, ENTRIES)
    };
    COUNTING.with(|flag| flag.set(false));
    assert_eq!(status, Status::Ok);
    assert_eq!(ALLOCATIONS.with(Cell::get), 0);
}

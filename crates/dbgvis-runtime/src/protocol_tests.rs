use super::*;

unsafe extern "C" fn unused_dispatch(_: usize) -> u64 {
    0
}

fn runtime(roots: Vec<Root>) -> &'static Runtime {
    let runtime = Box::leak(Box::new(Runtime::new()));
    let module = Box::leak(Box::new(Module::new(runtime, unused_dispatch)));
    runtime.enable(module, || roots);
    assert_eq!(module.ready.load(Ordering::Acquire), 1);
    runtime
}

unsafe fn call(runtime: &Runtime, request: Request) -> Response {
    unsafe {
        *runtime.request.get() = request;
        let status = runtime.dispatch(runtime.request.get() as usize);
        let response = *runtime.response.get();
        assert_eq!(response.sequence, request.sequence);
        assert_eq!(response.status, status);
        response
    }
}

#[test]
fn fixed_abi_and_capacity() {
    assert_eq!(size_of::<Module>(), 128);
    assert_eq!(size_of::<Entry>(), 96);
    assert_eq!(size_of::<Request>(), 80);
    assert_eq!(size_of::<Response>(), 40);
    assert_eq!(capacity(Some("131072")), 131072);
    for invalid in ["", "0", "-1", "x", "16777217"] {
        assert!(std::panic::catch_unwind(|| capacity(Some(invalid))).is_err());
    }
}

#[test]
fn validation_happens_before_object_access() {
    let runtime = runtime(vec![
        Registration::<u64>::new::<u64>("anchor").debug().finish(),
    ]);
    let value = 42u64;
    let good = Request {
        object: &value as *const _ as u64,
        sequence: 19,
        ..Request::EMPTY
    };
    let response = unsafe { call(runtime, good) };
    assert_eq!(response.status, Status::Ok as u64);
    assert_eq!(response.written, 2);
    let invalid = [
        Request { abi: 1, ..good },
        Request { size: 79, ..good },
        Request { slot: 999, ..good },
        Request { object: 0, ..good },
        Request { object: 1, ..good },
        Request {
            capacity: 0,
            ..good
        },
        Request {
            capacity: OUTPUT_CAPACITY as u64 + 1,
            ..good
        },
        Request {
            alternate: 2,
            ..good
        },
        Request {
            max_depth: 0,
            ..good
        },
        Request {
            max_depth: 129,
            ..good
        },
        Request {
            max_nodes: 0,
            ..good
        },
        Request {
            max_nodes: 1_000_001,
            ..good
        },
        Request {
            mode: DISPLAY,
            ..good
        },
    ];
    for request in invalid {
        let response = unsafe { call(runtime, request) };
        assert_ne!(response.status, Status::Ok as u64);
        assert_eq!(
            response.written, 0,
            "never publish previous output as success"
        );
    }
    assert_eq!(unsafe { runtime.dispatch(1) }, Status::Invalid as u64);
    runtime.busy.store(true, Ordering::Release);
    assert_eq!(
        unsafe { runtime.dispatch(runtime.request.get() as usize) },
        Status::Busy as u64
    );
    runtime.busy.store(false, Ordering::Release);
    assert_eq!(unsafe { call(runtime, good) }.status, Status::Ok as u64);
    let cold = Runtime::new();
    assert_eq!(unsafe { call(&cold, good) }.status, Status::NotReady as u64);
}

struct Value(u8);
impl Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            1 => panic!("formatter panic"),
            2 => Err(std::fmt::Error),
            _ => f.write_str("你好\0world"),
        }
    }
}

#[test]
fn errors_limits_nul_and_recovery() {
    let runtime = runtime(vec![
        Registration::<Value>::new::<Value>("anchor")
            .display()
            .finish(),
    ]);
    let mut value = Value(0);
    let request = Request {
        object: &value as *const _ as u64,
        ..Request::EMPTY
    };
    let response = unsafe { call(runtime, request) };
    assert_eq!(response.status, Status::Ok as u64);
    assert_eq!(
        unsafe { &(&*runtime.output.get())[..response.written as usize] },
        "你好\0world".as_bytes()
    );
    let response = unsafe {
        call(
            runtime,
            Request {
                capacity: 4,
                ..request
            },
        )
    };
    assert_eq!(response.status, Status::Limited as u64);
    assert_eq!(response.written, 3);
    for (kind, status) in [
        (1, Status::Panic),
        (2, Status::FormatError),
        (0, Status::Ok),
    ] {
        value.0 = kind;
        std::hint::black_box(&value);
        let response = unsafe { call(runtime, request) };
        assert_eq!(response.status, status as u64);
        if kind != 0 {
            assert_eq!(response.written, 0);
        }
        assert!(!runtime.busy.load(Ordering::Acquire));
    }
}

#[test]
fn duplicate_same_type_registration_is_coalesced() {
    let root = Registration::<u32>::new::<u32>("anchor").debug().finish();
    let other_marker = Registration::<u32>::new::<i32>("other").display().finish();
    let runtime = runtime(vec![root, other_marker]);
    let entries = runtime.entries.get().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].capabilities, DEBUG | DISPLAY);
}

#[test]
fn duplicate_type_name_with_different_shape_is_rejected() {
    let mut roots = vec![Registration::<u32>::new::<u32>("anchor").debug().finish()];
    let mut incompatible = Registration::<u32>::new::<i64>("other").debug().finish();
    // Even when identity is correct, inconsistent metadata must be rejected.
    incompatible.entry.size = 8;
    roots.push(incompatible);
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| coalesce_roots(&mut roots)))
            .is_err()
    );
}

#[test]
fn same_name_and_layout_do_not_prove_type_identity() {
    // Function-local types can have identical type_name output even within one crate.
    let first = {
        #[derive(Debug)]
        struct Same(u32);
        let value = Same(1);
        std::hint::black_box(value.0);
        Registration::<Same>::new::<u32>("a").debug().finish()
    };
    let second = {
        #[derive(Debug)]
        struct Same(f32);
        let value = Same(1.0);
        std::hint::black_box(value.0);
        Registration::<Same>::new::<f32>("b").debug().finish()
    };
    assert_eq!(first.name, second.name);
    assert_eq!(
        (first.entry.size, first.entry.align),
        (second.entry.size, second.entry.align)
    );
    assert_ne!(first.identity, second.identity);
    assert!(std::panic::catch_unwind(|| runtime(vec![first, second])).is_err());
}

#[test]
fn borrowed_identity_merges_modes_without_extending_values() {
    fn roots<'a>(_: &'a str) -> Vec<Root> {
        // SAFETY: &'static str is exactly &'a str with its lifetime erased.
        let a = unsafe { Registration::<&'a str>::new_lifetime_erased::<&'static str, u32>("a") };
        let b = unsafe { Registration::<&'a str>::new_lifetime_erased::<&'static str, u64>("b") };
        vec![a.debug().finish(), b.display().finish()]
    }
    let owned = String::from("borrowed");
    let value = owned.as_str();
    let runtime = runtime(roots(value));
    assert_eq!(runtime.entries.get().unwrap().len(), 1);
    for (mode, expected) in [(DEBUG, "\"borrowed\""), (DISPLAY, "borrowed")] {
        let response = unsafe {
            call(
                runtime,
                Request {
                    object: &value as *const _ as u64,
                    mode,
                    ..Request::EMPTY
                },
            )
        };
        assert_eq!(response.status, Status::Ok as u64);
        assert_eq!(
            unsafe { &(&*runtime.output.get())[..response.written as usize] },
            expected.as_bytes()
        );
    }
}

#[test]
fn duplicate_marker_for_different_types_is_rejected() {
    let marker = Registration::<u32>::new::<u32>("anchor").debug().finish();
    let mut other = Registration::<u64>::new::<u32>("other").debug().finish();
    other.name = "u64";
    assert!(std::panic::catch_unwind(|| coalesce_roots(&mut vec![marker, other])).is_err());
}

#[test]
fn an_empty_linked_registry_is_ready_but_has_no_callable_slot() {
    let runtime = runtime(vec![]);
    assert!(runtime.entries.get().unwrap().is_empty());
    let value = 7u32;
    let response = unsafe {
        call(
            runtime,
            Request {
                object: &value as *const _ as u64,
                ..Request::EMPTY
            },
        )
    };
    assert_eq!(response.status, Status::Invalid as u64);
    assert_eq!(response.written, 0);
}

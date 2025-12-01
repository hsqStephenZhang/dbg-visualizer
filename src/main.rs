#![debugger_visualizer(gdb_script_file = "printer.py")]

use bytes::Bytes;
use indexmap::IndexMap;
use std::ffi::{CString, c_char};
use std::fmt::Write;

#[unsafe(no_mangle)]
pub extern "C" fn debug_print_bytes(addr: usize) -> *const c_char {
    if addr == 0 {
        let s = CString::new("<null>").unwrap();
        return s.into_raw();
    }
    let ptr = addr as *const Bytes;
    let foo = unsafe { &*ptr };
    let mut s = String::new();
    write!(&mut s, "{:?}", foo).unwrap();
    let cstr = CString::new(s).unwrap();
    cstr.into_raw()
}

#[unsafe(no_mangle)]
pub extern "C" fn debug_print_indexmap_i32_str(addr: usize) -> *const c_char {
    unsafe {
        if addr == 0 {
            let s = CString::new("<null>").unwrap();
            return s.into_raw();
        }

        let ptr = addr as *const IndexMap<i32, &str>;
        let map = &*ptr;

        let formatted = format!("{:?}", map);
        let s = CString::new(formatted).unwrap();
        s.into_raw()
    }
}

#[unsafe(no_mangle)]
extern "C" fn debug_print_free(s: usize) {
    if s != 0 {
        unsafe {
            let _ = CString::from_raw(s as *mut c_char);
        }
    }
}

#[used]
static DEBUG_PRINT_INDEXMAP: extern "C" fn(usize) -> *const c_char = debug_print_indexmap_i32_str;
#[used]
static DEBUG_PRINT_BYTES: extern "C" fn(usize) -> *const c_char = debug_print_bytes;
#[used]
static DEBUG_PRINT_FREE: extern "C" fn(usize) = debug_print_free;

fn main() {
    let map = IndexMap::from([(1, "one"), (2, "two"), (3, "three")]);
    let bytes = Bytes::from("hello world");
    println!("{:?}", map);
    println!("{:?}", bytes);
    println!("map breakpoint here");
}

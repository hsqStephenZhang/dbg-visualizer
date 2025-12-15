## usage 

for gdb, install `https://github.com/cyrus-and/gdb-dashboard` and add project path to `safe-path`

## How does it work

rust has [debugger_visualizer](https://rust-lang.github.io/rfcs/3191-debugger-visualizer.html) for customize debugger view
we could provide ffi functions for display rust Structs that has implemented Debug trait

for `IndexMap<i32, &str>` we could generate such functions:

```rust
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
```

and `debug_print_free` **must** be called to deallocate the CString we just allocated after the debugger has got the actual display string.

notice that we use `usize` as the address's argument type, which is a work-around for lldb, since its type conversion is pretty annoying and it's the simpliest way i could work it out.

in lldb, we could see(the difference between `v` and `p` is that `p` cannot access the memory of the variable, so we could not evalute rust's ffi function but invoke the default printer method):

```txt
v map 
(indexmap::map::IndexMap<int, &str, std::hash::random::RandomState>) map = "{1: \"one\", 2: \"two\", 3: \"three\"}"
p map 
(indexmap::map::IndexMap<int, &str, std::hash::random::RandomState>) { core = { indices = { raw = { table = { bucket_mask = 3, ctrl = { pointer = "\U00000002\U00000015\xffT\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\U00000002\U00000015\xffT" }, growth_left = 0, items = 3 }, alloc = {}, marker = {} } }, entries = size=3 }, hash_builder = { k0 = 11368365513805002518, k1 = 11841700729499363736 } }
```

## how to use

to use that in gdb, the `debugger_visualizer` at the beginning of `main.rs` has already written the gdb script into a section of the final binary, and gdb/rust-gdb will recognize that, just call `print bytes` or `print map`

for lldb, we have to write another script since it does not provide such extension for binary, we have to mannual load the script by `command script import src/lldb_linter.py`, or you could write this command into `.lldbinit` file and use `command source .lldbinit`. see [lldb python api](https://lldb.llvm.org/use/variable.html#python-scripting), after that, call `v map` or `p map` to see the difference.

for codelldb in vscode, we could add such command to its `initCommands` in `launch.json`.

Wala!

## TODOs

there are many things we could work on top this POC.
since the `debug_print_xxx`'s argument is usize, there is nothing prevent you from invoking it with some arbituary pointer.
to make it more solid, we should add type checking in gdb scripts. for lldb, we could simply rely on its regex's matching like 
`type summary add "bytes::bytes::Bytes" -F lldb_linter.generic_summary_provider -x -h "^bytes::bytes::Bytes$" --category Rust` which 
will only we called if the full path of the type matches the regex pattern. 

and we could provide some proc-macros to generate these gdb/lldb script automatically, saving the trouble of hardcoding them(trust me, they are pretty trivial but boring)

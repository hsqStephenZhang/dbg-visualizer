use bytes::Bytes;
use indexmap::IndexMap;
use visualizer_adapters::{BytesElements, IndexMapEntries};
use visualizer_runtime::{Child, Children, Status};

#[test]
fn bytes_pages_borrow_actual_slice_storage() {
    let bytes = Bytes::from("abcdef");
    let slice = bytes.slice(2..5);
    let mut out = [Child::EMPTY; 8];
    assert_eq!(BytesElements::count(&slice), 3);
    assert_eq!(BytesElements::page(&slice, 1, 2, &mut out).unwrap(), 2);
    assert_eq!(out[0].address, unsafe { bytes.as_ptr().add(3) } as u64);
    assert_eq!(unsafe { *(out[1].address as *const u8) }, b'e');
    assert_eq!(
        BytesElements::page(&slice, 3, 1, &mut out),
        Err(Status::InvalidRequest)
    );
    assert_eq!(
        BytesElements::page(&Bytes::new(), 0, 0, &mut out).unwrap(),
        0
    );
}

#[test]
fn map_pages_preserve_keys_order_and_nonstatic_strings() {
    let owned = String::from("owned text");
    let mut map = IndexMap::from([(5, "first"), (2, owned.as_str()), (7, "last")]);
    let mut out = [Child::EMPTY; 8];
    assert_eq!(IndexMapEntries::page(&map, 1, 2, &mut out).unwrap(), 4);
    assert_eq!(unsafe { *(out[0].address as *const i32) }, 2);
    assert_eq!(unsafe { *(out[1].address as *const &str) }, owned);
    assert_eq!(out[1].kind, 2);
    assert_eq!(out[1].length, owned.len() as u64);
    map.shift_remove(&5);
    map.insert(9, "new");
    IndexMapEntries::page(&map, 0, 3, &mut out).unwrap();
    assert_eq!(
        (0..3)
            .map(|i| unsafe { *(out[i * 2].address as *const i32) })
            .collect::<Vec<_>>(),
        [2, 7, 9]
    );
    assert_eq!(
        IndexMapEntries::page(&map, usize::MAX, 1, &mut out),
        Err(Status::InvalidRequest)
    );
}

use bytes::Bytes;
use indexmap::IndexMap;
use visualizer_runtime::{Child, Children, MAP, SEQUENCE, Status};

pub struct BytesElements;

// Bytes indexing borrows the actual byte storage, including the slice offset.
unsafe impl Children<Bytes> for BytesElements {
    const SHAPE: u64 = SEQUENCE;
    fn count(value: &Bytes) -> usize {
        value.len()
    }
    fn page(value: &Bytes, start: usize, count: usize, out: &mut [Child]) -> Result<usize, Status> {
        let slice = value
            .get(start..start.checked_add(count).ok_or(Status::InvalidRequest)?)
            .ok_or(Status::InvalidRequest)?;
        if out.len() < count {
            return Err(Status::InvalidRequest);
        }
        for (destination, byte) in out.iter_mut().zip(slice) {
            *destination = Child::borrowed("", byte)?;
        }
        Ok(count)
    }
}

pub struct IndexMapEntries;

// No private hash table layout assumptions: get_index returns references to stored K/V slots.
// The lifetime is generic; strings need not be static and references never escape the call
// as Rust references. The debugger must invalidate descriptor addresses when execution resumes.
unsafe impl<'a> Children<IndexMap<i32, &'a str>> for IndexMapEntries {
    const SHAPE: u64 = MAP;
    fn count(value: &IndexMap<i32, &'a str>) -> usize {
        value.len()
    }
    fn page(
        value: &IndexMap<i32, &'a str>,
        start: usize,
        count: usize,
        out: &mut [Child],
    ) -> Result<usize, Status> {
        if out.len() < count.checked_mul(2).ok_or(Status::InvalidRequest)? {
            return Err(Status::InvalidRequest);
        }
        let end = start.checked_add(count).ok_or(Status::InvalidRequest)?;
        if end > value.len() {
            return Err(Status::InvalidRequest);
        }
        for (slot, index) in (start..end).enumerate() {
            let (key, value) = value.get_index(index).ok_or(Status::InvalidRequest)?;
            out[slot * 2] = Child::borrowed("key", key)?;
            out[slot * 2 + 1] = Child::string("value", value)?;
        }
        Ok(count * 2)
    }
}

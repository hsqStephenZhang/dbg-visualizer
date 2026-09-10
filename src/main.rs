#[cfg(feature = "visualizer")]
include!(concat!(env!("OUT_DIR"), "/autoload.rs"));

use bytes::Bytes;
use indexmap::IndexMap;
use std::fmt;

#[derive(Debug, Clone, Copy)]
struct Point {
    x: i32,
    y: i32,
}

impl fmt::Display for Point {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        if out.alternate() {
            write!(out, "坐标({}, {})", self.x, self.y)
        } else {
            write!(out, "({}, {})", self.x, self.y)
        }
    }
}

#[derive(Debug)]
struct Record {
    point: Point,
    bytes: Bytes,
    next: *const Record,
}

#[cfg(feature = "visualizer")]
struct PointFields;
#[cfg(feature = "visualizer")]
unsafe impl visualizer_runtime::Children<Point> for PointFields {
    const SHAPE: u64 = visualizer_runtime::STRUCT;
    fn count(_: &Point) -> usize {
        2
    }
    fn page(
        value: &Point,
        start: usize,
        count: usize,
        out: &mut [visualizer_runtime::Child],
    ) -> Result<usize, visualizer_runtime::Status> {
        use visualizer_runtime::{Child, Status};
        let fields = [
            Child::borrowed("x", &value.x)?,
            Child::borrowed("y", &value.y)?,
        ];
        let page = fields
            .get(start..start.checked_add(count).ok_or(Status::InvalidRequest)?)
            .ok_or(Status::InvalidRequest)?;
        out[..count].copy_from_slice(page);
        Ok(count)
    }
}

#[cfg(feature = "visualizer")]
struct RecordFields;
#[cfg(feature = "visualizer")]
unsafe impl visualizer_runtime::Children<Record> for RecordFields {
    const SHAPE: u64 = visualizer_runtime::STRUCT;
    fn count(_: &Record) -> usize {
        3
    }
    fn page(
        value: &Record,
        start: usize,
        count: usize,
        out: &mut [visualizer_runtime::Child],
    ) -> Result<usize, visualizer_runtime::Status> {
        use visualizer_runtime::{Child, Status};
        let fields = [
            Child::borrowed("point", &value.point)?,
            Child::borrowed("bytes", &value.bytes)?,
            Child::borrowed("next", &value.next)?,
        ];
        let page = fields
            .get(start..start.checked_add(count).ok_or(Status::InvalidRequest)?)
            .ok_or(Status::InvalidRequest)?;
        out[..count].copy_from_slice(page);
        Ok(count)
    }
}

#[cfg(feature = "visualizer")]
visualizer_runtime::register_visualizers! {
    Bytes => { name: "bytes::bytes::Bytes", debug, children: visualizer_adapters::BytesElements },
    IndexMap<i32, &str> => {
        name: "indexmap::map::IndexMap<i32, &str>",
        aliases: ["indexmap::map::IndexMap<i32, &str, std::hash::random::RandomState>",
                  "indexmap::map::IndexMap<int, &str, std::hash::random::RandomState>"],
        debug, children: visualizer_adapters::IndexMapEntries
    },
    Point => { name: "dbg_visualizer::Point", debug, display, children: PointFields },
    Record => { name: "dbg_visualizer::Record", debug, children: RecordFields }
}

#[inline(never)]
fn checkpoint(stage: u32) {
    std::hint::black_box(stage);
}

fn main() {
    #[cfg(feature = "visualizer")]
    visualizer_runtime::retain(&DBG_VIS_MODULE_V1);
    let owned = String::from("临时字符串");
    let mut map = IndexMap::from([(1, "one"), (2, owned.as_str()), (3, "three")]);
    let bytes = Bytes::from("hello world");
    let sliced = bytes.slice(6..);
    let empty = Bytes::new();
    let large = Bytes::from(vec![b'x'; 10_000]);
    let point = Point { x: 3, y: 7 };
    let points: [Point; 128] = std::array::from_fn(|i| Point { x: i as i32, y: 7 });
    let mut record = Record {
        point: Point { x: 4, y: 8 },
        bytes: bytes.clone(),
        next: std::ptr::null(),
    };
    record.next = &record;
    checkpoint(0);
    map.insert(4, "four");
    map.shift_remove(&1);
    checkpoint(1);
    std::hint::black_box((&map, &bytes, &sliced, &empty, &large, &point, &record));
    std::hint::black_box((&record.point, &record.bytes));
    std::hint::black_box(&points);
}

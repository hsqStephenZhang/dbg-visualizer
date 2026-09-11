use std::collections::HashMap;
use std::fmt;
use std::hash::{BuildHasherDefault, DefaultHasher};
use std::marker::PhantomData;

struct HasherPolicy;
impl std::hash::BuildHasher for HasherPolicy {
    type Hasher = DefaultHasher;
    fn build_hasher(&self) -> DefaultHasher {
        DefaultHasher::new()
    }
}

#[cfg_attr(feature = "visualize", derive(dbgvis::Visualize))]
struct Point {
    x: i32,
    y: i32,
}
impl fmt::Display for Point {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(out, "({}, {})", self.x, self.y)
    }
}

struct NoTraits;
#[cfg_attr(feature = "visualize", derive(dbgvis::Visualize))]
struct Borrowed<'a, T, Policy, const N: usize> {
    label: &'a str,
    value: T,
    marker: PhantomData<Policy>,
}

#[cfg_attr(feature = "visualize", derive(dbgvis::Visualize))]
enum State<T> {
    Ready { value: T },
    Pending,
}

#[cfg_attr(feature = "visualize", derive(dbgvis::Visualize))]
struct AppState {
    library_a: demo_a::Record<demo_b::Item, NoTraits>,
    library_c: demo_c::State,
    label: demo_c::Label,
    points: HashMap<String, Vec<Option<Point>>>,
    bytes: bytes::Bytes,
    ordered: indexmap::IndexMap<u32, String>,
    address: std::net::SocketAddr,
    state: State<u32>,
    #[cfg_attr(feature = "visualize", dbgvis(skip))]
    secret: NoTraits,
}

#[cfg(feature = "visualize")]
#[dbgvis::visualizers]
mod visualizers {
    #[dbgvis(visualize)]
    type App = super::AppState;
    #[dbgvis(visualize)]
    type Map = std::collections::HashMap<String, Vec<Option<super::Point>>>;
    #[dbgvis(visualize)]
    type CustomMap = std::collections::HashMap<(), u64, super::HasherPolicy>;
    #[dbgvis(visualize, display)]
    type Point = super::Point;
    #[dbgvis(auto, debug)]
    type Bytes = bytes::Bytes;
    #[dbgvis(auto)]
    type IndexMap = indexmap::IndexMap<u32, String>;
    #[dbgvis(auto)]
    type IndexBorrowed<'a> =
        indexmap::IndexMap<&'a str, u64, std::hash::BuildHasherDefault<std::hash::DefaultHasher>>;
    #[dbgvis(auto)]
    type IndexArray = indexmap::IndexMap<[u8; 2], Vec<()>>;
    type Hashbrown =
        hashbrown::HashMap<u32, Vec<u8>, std::hash::BuildHasherDefault<std::hash::DefaultHasher>>;
    #[dbgvis(auto, debug, display)]
    type Address = std::net::SocketAddr;
    #[dbgvis(visualize)]
    type Borrowed<'a> = super::Borrowed<'a, Vec<u8>, super::NoTraits, 17>;
    #[dbgvis(visualize)]
    type BorrowedOther<'a> = super::Borrowed<'a, Vec<u8>, super::NoTraits, 19>;
}

#[inline(never)]
fn checkpoint(stage: u32, app: &AppState) {
    // Materialize the complete value before the breakpoint, including under LTO.
    std::hint::black_box((stage, app));
}

fn main() {
    #[cfg(feature = "visualize")]
    dbgvis::enable!(visualizers);
    let text = String::from("临时字符串");
    let point = Point { x: 3, y: 7 };
    let map = HashMap::from([(
        String::from("points"),
        vec![Some(Point { x: 1, y: 2 }), None],
    )]);
    let mut custom = HashMap::<(), u64, HasherPolicy>::with_hasher(HasherPolicy);
    custom.insert((), 42);
    let bytes = bytes::Bytes::from("hello world");
    let ordered = indexmap::IndexMap::from([(1u32, String::from("one")), (2, String::from("two"))]);
    let address: std::net::SocketAddr = "127.0.0.1:8080".parse().unwrap();
    let mut index_borrowed =
        indexmap::IndexMap::<&str, u64, BuildHasherDefault<DefaultHasher>>::default();
    index_borrowed.insert(&text, 123);
    let index_array = indexmap::IndexMap::from([([1u8, 2], vec![(), ()])]);
    let mut external_map =
        hashbrown::HashMap::<u32, Vec<u8>, BuildHasherDefault<DefaultHasher>>::default();
    external_map.insert(9, vec![8, 7]);
    let borrowed = Borrowed::<_, NoTraits, 17> {
        label: &text,
        value: vec![1u8, 2],
        marker: PhantomData,
    };
    let borrowed_other = Borrowed::<_, NoTraits, 19> {
        label: &text,
        value: vec![3u8, 4],
        marker: PhantomData,
    };
    let mut app = AppState {
        library_a: demo_a::Record::new(demo_b::Item::new(11)),
        library_c: demo_c::State::Ready {
            message: "跨 crate".into(),
        },
        label: demo_c::Label::new("Display-only"),
        points: HashMap::from([(String::from("app"), vec![Some(Point { x: 5, y: 6 })])]),
        bytes: bytes.clone(),
        ordered: ordered.clone(),
        address,
        state: State::Ready { value: 7 },
        secret: NoTraits,
    };
    checkpoint(0, &app);
    app.state = State::Pending;
    app.ordered.insert(3, String::from("three"));
    checkpoint(1, &app);
    std::hint::black_box((
        &app,
        &map,
        &custom,
        &point,
        &bytes,
        &ordered,
        &address,
        &borrowed,
        &borrowed_other,
        &index_borrowed,
        &index_array,
        &external_map,
    ));
    // These reads keep demo fields useful even when visualization is compiled out.
    std::hint::black_box((
        &app.points,
        &app.bytes,
        &app.address,
        &app.secret,
        &app.library_a,
        &app.library_c,
        &app.label,
        &borrowed.label,
        &borrowed.value,
    ));
    if let State::Ready { value } = &app.state {
        std::hint::black_box(value);
    }
}

use dbgvis::Visualize;
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, DefaultHasher};
use std::marker::PhantomData;

struct HasherPolicy;
impl std::hash::BuildHasher for HasherPolicy {
    type Hasher = DefaultHasher;
    fn build_hasher(&self) -> DefaultHasher {
        DefaultHasher::new()
    }
}

#[derive(Debug)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug)]
struct Marker;

#[derive(Debug)]
struct Counter {
    count: u32,
}
#[derive(Debug)]
struct Borrowed<'a, T, Policy, const N: usize> {
    label: &'a str,
    value: T,
    marker: PhantomData<Policy>,
}

#[derive(Debug)]
enum State<T> {
    Ready { value: T },
    Pending,
}

#[derive(Debug)]
struct AppState {
    points: HashMap<String, Vec<Option<Point>>>,
    bytes: bytes::Bytes,
    ordered: indexmap::IndexMap<u32, String>,
    address: std::net::SocketAddr,
    state: State<u32>,
    secret: Marker,
}

#[inline(never)]
fn checkpoint(stage: u32, app: &AppState) {
    // Materialize the complete value before the breakpoint, including under LTO.
    std::hint::black_box((stage, app));
}

fn main() {
    dbgvis::enable!();
    #[derive(Visualize)]
    struct Local {
        value: u32,
    }
    let local = Local { value: 17 };
    let counter = Counter { count: 23 };
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
    let borrowed = Borrowed::<_, Marker, 17> {
        label: &text,
        value: vec![1u8, 2],
        marker: PhantomData,
    };
    let borrowed_other = Borrowed::<_, Marker, 19> {
        label: &text,
        value: vec![3u8, 4],
        marker: PhantomData,
    };
    let mut app = AppState {
        points: HashMap::from([(String::from("app"), vec![Some(Point { x: 5, y: 6 })])]),
        bytes: bytes.clone(),
        ordered: ordered.clone(),
        address,
        state: State::Ready { value: 7 },
        secret: Marker,
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
        &local,
        &counter,
    ));
    // Keep skipped fields and other demo values live for the debugger.
    std::hint::black_box((
        &app.points,
        &app.bytes,
        &app.address,
        &app.secret,
        &borrowed.label,
        &borrowed.value,
        &local.value,
        &counter.count,
    ));
    if let State::Ready { value } = &app.state {
        std::hint::black_box(value);
    }
}

use dbgvis::{FormatOptions, Formatter, Outcome, Visualize, format_into};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fmt::{self, Write};
use std::hash::{BuildHasherDefault, DefaultHasher};
use std::marker::PhantomData;

fn text<T: ?Sized>(value: &T) -> String {
    let mut buffer = [0; 4096];
    let output = format_into(value, &mut buffer, FormatOptions::default());
    assert_eq!(output.outcome, Outcome::Complete);
    output.text.to_owned()
}

#[derive(Visualize)]
struct OnlyVisual {
    value: u32,
}
#[derive(Debug)]
struct OnlyDebug;
struct OnlyDisplay;
impl fmt::Display for OnlyDisplay {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str("display-only")
    }
}
#[derive(Visualize)]
struct Preferred {
    value: u32,
}
impl fmt::Debug for Preferred {
    fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
        panic!("Debug must not mask Visualize")
    }
}
impl fmt::Display for Preferred {
    fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
        panic!("Display must not mask Visualize")
    }
}

#[test]
fn recursive_auto_selection_is_generic() {
    assert_eq!(
        text(&vec![OnlyVisual { value: 7 }]),
        "[OnlyVisual { value: 7 }]"
    );
    assert_eq!(text(&vec![OnlyDebug]), "[OnlyDebug]");
    assert_eq!(text(&vec![OnlyDisplay]), "[display-only]");
    assert_eq!(
        text(&HashMap::from([(1, vec![Preferred { value: 9 }])])),
        "{1: [Preferred { value: 9 }]}"
    );
    assert_eq!(
        text(&HashMap::from([(1, OnlyDisplay)])),
        "{1: display-only}"
    );
    assert_eq!(
        text(&BTreeMap::from([(1, Some(OnlyVisual { value: 2 }))])),
        "{1: Some(OnlyVisual { value: 2 })}"
    );
    assert_eq!(text(&VecDeque::from([1, 2])), "[1, 2]");
    assert_eq!(text(&HashSet::from([()])), "[()]");
}

struct NoTraits;
#[derive(Visualize)]
struct Borrowed<'a, T, P, const N: usize> {
    value: &'a T,
    marker: PhantomData<P>,
    #[dbgvis(skip)]
    secret: NoTraits,
}
#[test]
fn borrowed_const_generic_and_zst_policy_need_no_extra_bounds() {
    let value = String::from("局部");
    let wrapper = Borrowed::<_, NoTraits, 17> {
        value: &value,
        marker: PhantomData,
        secret: NoTraits,
    };
    let _ = &wrapper.secret;
    assert_eq!(
        text(&wrapper),
        "Borrowed { value: \"局部\", marker: PhantomData }"
    );
    let mut map = HashMap::<(), OnlyDisplay, BuildHasherDefault<DefaultHasher>>::default();
    map.insert((), OnlyDisplay);
    assert_eq!(text(&map), "{(): display-only}");
    assert_eq!(text(&[(); 3]), "[(), (), ()]");
}

#[derive(Visualize)]
#[dbgvis(rename = "Visible")]
struct Override<T> {
    #[dbgvis(rename = "shown", via = "display")]
    value: T,
}
#[derive(Visualize)]
#[dbgvis(transparent)]
struct Transparent<T>(T, #[dbgvis(skip)] NoTraits);
#[derive(Visualize)]
enum Enum<T> {
    Unit,
    Tuple(T, #[dbgvis(skip)] NoTraits),
    Named { value: T },
}
#[test]
fn field_attributes_variants_and_transparency() {
    assert_eq!(text(&Override { value: 5 }), "Visible { shown: 5 }");
    assert_eq!(text(&Transparent(OnlyDisplay, NoTraits)), "display-only");
    assert_eq!(text(&Enum::<u8>::Unit), "Enum::Unit");
    assert_eq!(
        text(&Enum::Tuple(OnlyDisplay, NoTraits)),
        "Enum::Tuple(display-only)"
    );
    assert_eq!(
        text(&Enum::Named {
            value: vec![OnlyDebug]
        }),
        "Enum::Named { value: [OnlyDebug] }"
    );
}

#[test]
fn byte_depth_node_budgets_and_utf8() {
    for length in 0..32 {
        let mut buffer = vec![0; length];
        let output = format_into(&"a中🙂\0end", &mut buffer, FormatOptions::default());
        assert!(output.text.len() <= length);
        assert!(std::str::from_utf8(output.text.as_bytes()).is_ok());
    }
    let mut buffer = [0; 256];
    let output = format_into(
        &vec![vec![1, 2]],
        &mut buffer,
        FormatOptions {
            max_depth: 1,
            ..Default::default()
        },
    );
    assert_eq!(output.outcome, Outcome::Depth);
    let output = format_into(
        &vec![1, 2, 3],
        &mut buffer,
        FormatOptions {
            max_nodes: 2,
            ..Default::default()
        },
    );
    assert_eq!(output.outcome, Outcome::Nodes);
    assert_eq!(output.nodes, 2);
    assert!(!output.text.contains('2'));
    let output = format_into(
        &OnlyVisual { value: 1 },
        &mut buffer,
        FormatOptions {
            alternate: true,
            ..Default::default()
        },
    );
    assert_eq!(output.text, "OnlyVisual {\n  value: 1\n}");
}

#[test]
fn cycles_stop_but_shared_values_and_zsts_are_not_global_duplicates() {
    use std::{cell::RefCell, rc::Rc};
    struct Node {
        next: RefCell<Option<Rc<Node>>>,
    }
    impl Visualize for Node {
        fn visualize(&self, out: &mut Formatter<'_>) -> dbgvis::Result {
            out.value(&*self.next.borrow())
        }
    }
    let node = Rc::new(Node {
        next: RefCell::new(None),
    });
    *node.next.borrow_mut() = Some(node.clone());
    let mut buffer = [0; 256];
    let output = format_into(&node, &mut buffer, FormatOptions::default());
    assert_eq!(output.outcome, Outcome::Cycle);
    node.next.borrow_mut().take();
    let shared = Rc::new(OnlyVisual { value: 3 });
    assert_eq!(
        text(&(shared.clone(), shared)),
        "(OnlyVisual { value: 3 }, OnlyVisual { value: 3 })"
    );
}

struct Failure;
impl Visualize for Failure {
    fn visualize(&self, _: &mut Formatter<'_>) -> dbgvis::Result {
        Err(fmt::Error)
    }
}
impl fmt::Debug for Failure {
    fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
        panic!("must not retry Debug")
    }
}
#[test]
fn no_lower_priority_retry_and_ignored_write_error_stays_truncated() {
    let mut buffer = [0; 16];
    assert_eq!(
        format_into(&Failure, &mut buffer, FormatOptions::default()).outcome,
        Outcome::FormatError
    );
    struct IgnoresError;
    impl Visualize for IgnoresError {
        fn visualize(&self, out: &mut Formatter<'_>) -> dbgvis::Result {
            let _ = out.write_str("abcdefghijklmnopq");
            Ok(())
        }
    }
    assert_eq!(
        format_into(&IgnoresError, &mut buffer, FormatOptions::default()).outcome,
        Outcome::Bytes
    );
}

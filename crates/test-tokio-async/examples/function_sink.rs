//! Installs a `FunctionSink` and checks the scopes it receives: every enter
//! has one exit with the same token, sync scopes nest per thread, and each
//! scope reports the expected `ScopeKind`.
//!
//! Run with:
//!   cargo run -p test-tokio-async --example function_sink --features hotpath-sink

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread::ThreadId;
use std::time::Duration;

use hotpath::{FunctionSink, ScopeKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Edge {
    Enter,
    Exit,
}

#[derive(Debug)]
struct Event {
    edge: Edge,
    name: &'static str,
    kind: ScopeKind,
    token: u64,
    thread: ThreadId,
}

static EVENTS: OnceLock<Mutex<Vec<Event>>> = OnceLock::new();

fn events() -> &'static Mutex<Vec<Event>> {
    EVENTS.get_or_init(|| Mutex::new(Vec::new()))
}

#[derive(Default)]
struct RecordingSink {
    next_token: AtomicU64,
}

impl RecordingSink {
    fn record(&self, edge: Edge, name: &'static str, kind: ScopeKind, token: u64) {
        events().lock().unwrap().push(Event {
            edge,
            name,
            kind,
            token,
            thread: std::thread::current().id(),
        });
    }
}

impl FunctionSink for RecordingSink {
    fn enter(&self, name: &'static str, kind: ScopeKind) -> u64 {
        let token = self.next_token.fetch_add(1, Ordering::Relaxed);
        self.record(Edge::Enter, name, kind, token);
        token
    }

    fn exit(&self, name: &'static str, kind: ScopeKind, token: u64) {
        self.record(Edge::Exit, name, kind, token);
    }
}

struct OtherSink;

impl FunctionSink for OtherSink {
    fn enter(&self, _name: &'static str, _kind: ScopeKind) -> u64 {
        0
    }

    fn exit(&self, _name: &'static str, _kind: ScopeKind, _token: u64) {}
}

#[hotpath::measure]
fn inner(i: u64) -> u64 {
    std::hint::black_box(i * 2)
}

#[hotpath::measure]
fn outer(i: u64) -> u64 {
    inner(i) + inner(i + 1)
}

#[hotpath::measure]
async fn async_work(i: u64) -> u64 {
    tokio::time::sleep(Duration::from_millis(1)).await;
    outer(i)
}

#[hotpath::measure(log = true)]
fn logged(i: u64) -> u64 {
    i + 1
}

fn verify(events: &[Event]) {
    let mut open: HashMap<u64, &Event> = HashMap::new();
    let mut sync_stacks: HashMap<ThreadId, Vec<u64>> = HashMap::new();

    for event in events {
        match event.edge {
            Edge::Enter => {
                assert!(
                    open.insert(event.token, event).is_none(),
                    "token {} entered twice",
                    event.token
                );
                if event.kind == ScopeKind::Sync {
                    sync_stacks
                        .entry(event.thread)
                        .or_default()
                        .push(event.token);
                }
            }
            Edge::Exit => {
                let enter = open
                    .remove(&event.token)
                    .unwrap_or_else(|| panic!("exit without enter: {event:?}"));
                assert_eq!(enter.name, event.name, "exit name differs from enter");
                assert_eq!(enter.kind, event.kind, "exit kind differs from enter");
                if event.kind == ScopeKind::Sync {
                    assert_eq!(enter.thread, event.thread, "sync scope crossed threads");
                    let top = sync_stacks.get_mut(&event.thread).and_then(Vec::pop);
                    assert_eq!(top, Some(event.token), "sync scopes not nested");
                }
            }
        }
    }
    assert!(open.is_empty(), "scopes never exited: {open:?}");

    let kind_of = |suffix: &str| {
        events
            .iter()
            .find(|e| e.name.ends_with(suffix))
            .map(|e| e.kind)
            .unwrap_or_else(|| panic!("no scope named *{suffix}"))
    };
    assert_eq!(kind_of("::outer"), ScopeKind::Sync);
    assert_eq!(kind_of("::inner"), ScopeKind::Sync);
    assert_eq!(kind_of("::logged"), ScopeKind::Sync);
    assert_eq!(kind_of("::async_work"), ScopeKind::Async);
    assert_eq!(kind_of("sink_block"), ScopeKind::Async);
    assert!(
        !events.iter().any(|e| e.name.ends_with("::main")),
        "the main wrapper started before the sink was installed"
    );

    let enters = |suffix: &str| {
        events
            .iter()
            .filter(|e| e.edge == Edge::Enter && e.name.ends_with(suffix))
            .count()
    };
    assert_eq!(enters("::outer"), 10 + 4);
    assert_eq!(enters("::inner"), 2 * (10 + 4));
    assert_eq!(enters("::async_work"), 4);
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
#[hotpath::main]
async fn main() {
    hotpath::set_function_sink(RecordingSink::default()).expect("first sink");

    for i in 0..10 {
        outer(i);
        logged(i);
        hotpath::measure_block!("sink_block", {
            std::hint::black_box(i);
        });
    }

    let tasks: Vec<_> = (0..4).map(|i| tokio::spawn(async_work(i))).collect();
    for task in tasks {
        task.await.unwrap();
    }

    let events = events().lock().unwrap();
    if cfg!(feature = "hotpath-sink") {
        assert_eq!(
            hotpath::set_function_sink(OtherSink),
            Err(hotpath::FunctionSinkAlreadySet)
        );
        verify(&events);
    } else {
        assert!(
            events.is_empty(),
            "sink called without the hotpath-sink feature"
        );
    }
    println!("function_sink ok: {} events", events.len());
}

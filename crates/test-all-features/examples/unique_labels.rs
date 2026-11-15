//! One literal label reused across every resource kind that accepts `label`,
//! proving uniqueness is enforced per kind, not globally. `measure_block!`
//! shares the functions namespace with `#[measure(label)]`, so it gets its own
//! label. Runtime label expressions are never checked and may repeat a literal.
//! Unlabeled entries sit beside them to check the report's `labeled` flag.
//!
//! Run with:
//!   cargo run -p test-all-features --example unique_labels --features hotpath

use futures::StreamExt;
use std::io::{Read, Write};

#[hotpath::measure(label = "shared")]
fn measured() {
    std::hint::black_box(0);
}

#[hotpath::measure]
fn unlabeled() {
    std::hint::black_box(0);
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let _guard = hotpath::HotpathGuardBuilder::new("main")
        .format(hotpath::Format::Json)
        .sections(vec![
            hotpath::Section::FunctionsTiming,
            hotpath::Section::Channels,
            hotpath::Section::Streams,
            hotpath::Section::Futures,
            hotpath::Section::RwLocks,
            hotpath::Section::Mutexes,
            hotpath::Section::Io,
        ])
        .build();

    measured();
    unlabeled();
    hotpath::measure_block!("shared-block", {
        std::hint::black_box(0);
    });
    let block_label: &'static str = std::hint::black_box("shared-block-runtime");
    hotpath::measure_block!(block_label, {
        std::hint::black_box(0);
    });

    let (tx, mut rx) = hotpath::channel!(tokio::sync::mpsc::channel::<u8>(4));
    tx.send(1).await.unwrap();
    rx.recv().await.unwrap();

    let (tx, mut rx) = hotpath::channel!(tokio::sync::mpsc::channel::<u8>(4), label = "shared");
    tx.send(1).await.unwrap();
    rx.recv().await.unwrap();

    let runtime_label = format!("shared-{}", "runtime");
    let (tx, mut rx) =
        hotpath::channel!(tokio::sync::mpsc::channel::<u8>(4), label = runtime_label);
    tx.send(1).await.unwrap();
    rx.recv().await.unwrap();

    let mut stream = hotpath::stream!(futures::stream::iter(0..3u8), label = "shared");
    while stream.next().await.is_some() {}

    hotpath::future!(async { 1u8 }, label = "shared").await;

    let mutex = hotpath::mutex!(tokio::sync::Mutex::new(0u8), label = "shared");
    *mutex.lock().await += 1;

    let rw_lock = hotpath::rw_lock!(tokio::sync::RwLock::new(0u8), label = "shared");
    *rw_lock.write().await += 1;

    let mut sink = hotpath::io!(Vec::new(), label = "shared");
    sink.write_all(b"abc").unwrap();
    let mut source = hotpath::io!(std::io::Cursor::new(vec![1u8, 2, 3]), label = "shared-read");
    let mut buf = Vec::new();
    source.read_to_end(&mut buf).unwrap();
}

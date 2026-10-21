//! Run with:
//!   cargo run -p test-tokio-async --example alloc_measure --features hotpath-alloc

fn uninstrumented_1kb() -> Vec<u8> {
    let buf = vec![0u8; 1024];
    std::hint::black_box(&buf);
    buf
}

#[hotpath::measure]
fn instrumented_1kb() {
    let buf = vec![0u8; 1024];
    std::hint::black_box(&buf);
}

#[hotpath::measure]
fn uninstrumented_children_2kb() {
    let a = uninstrumented_1kb();
    let b = uninstrumented_1kb();
    std::hint::black_box((&a, &b));
}

#[hotpath::measure]
fn own_1kb_plus_uninstrumented_child_1kb() {
    let own = vec![0u8; 1024];
    std::hint::black_box(&own);
    let child = uninstrumented_1kb();
    std::hint::black_box(&child);
}

#[hotpath::measure]
fn own_1kb_plus_uninstrumented_1kb_plus_instrumented_1kb() {
    let own = vec![0u8; 1024];
    std::hint::black_box(&own);
    let from_uninstrumented = uninstrumented_1kb();
    std::hint::black_box(&from_uninstrumented);
    instrumented_1kb();
}

#[hotpath::main]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    uninstrumented_children_2kb();
    own_1kb_plus_uninstrumented_child_1kb();
    own_1kb_plus_uninstrumented_1kb_plus_instrumented_1kb();

    Ok(())
}

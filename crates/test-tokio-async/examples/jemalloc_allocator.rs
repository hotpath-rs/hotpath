//! Run with:
//!   cargo run -p test-tokio-async --example jemalloc_allocator --features hotpath-alloc,jemalloc-alloc

#[hotpath::measure]
fn alloc_work() {
    let buf = vec![0u8; 4096];
    std::hint::black_box(&buf);
}

#[hotpath::main(allocator = tikv_jemallocator::Jemalloc)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    for _ in 0..100 {
        alloc_work();
    }

    Ok(())
}

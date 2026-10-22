//! Run with:
//!   cargo run -p test-tokio-async --example multithread_alloc --features hotpath-alloc

use std::thread;
use std::time::Duration;

#[hotpath::measure]
fn allocate_and_work(thread_id: usize, iterations: usize) {
    for i in 0..iterations {
        let vec1 = vec![thread_id; 100];
        std::hint::black_box(&vec1);

        let vec2 = vec![i; 1024];
        std::hint::black_box(&vec2);

        let s = format!("Thread {} iteration {}", thread_id, i);
        std::hint::black_box(&s);

        thread::sleep(Duration::from_micros(1));
    }
}

#[hotpath::measure]
fn nested_allocations(depth: usize) {
    let data = vec![depth; 512];
    std::hint::black_box(&data);

    if depth > 0 {
        nested_allocations(depth - 1);
    }
}

#[hotpath::measure]
fn run_thread_work(thread_id: usize) {
    allocate_and_work(thread_id, 50);
    nested_allocations(5);
}

#[hotpath::main]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    const NUM_THREADS: usize = 14;

    let handles: Vec<_> = (0..NUM_THREADS)
        .map(|i| {
            thread::spawn(move || {
                run_thread_work(i);
            })
        })
        .collect();

    run_thread_work(99);

    for handle in handles {
        handle.join().expect("Thread panicked");
    }

    println!("All threads completed successfully");

    Ok(())
}

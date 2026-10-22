//! Validates CPU attribution for free functions, `measure_all` impls, and
//! bare `measure(impl_type = ...)` on inherent impl methods.
//!
//! Run:
//! ```bash
//! cargo run -p test-tokio-async --example cpu_symbols \
//!   --features 'hotpath-cpu' --release
//! ```

use std::hint::black_box;

#[hotpath::measure]
#[inline(never)]
fn free_heavy_work(iterations: u32) -> u64 {
    let mut result: u64 = 1;
    for i in 0..iterations {
        result = result.wrapping_mul(black_box(i as u64).wrapping_add(7));
        result ^= result >> 3;
    }
    result
}

struct Worker;

#[hotpath::measure_all]
impl Worker {
    #[inline(never)]
    fn method_heavy_work(iterations: u32) -> u64 {
        let mut result: u64 = 1;
        for i in 0..iterations {
            result = result.wrapping_mul(black_box(i as u64).wrapping_add(7));
            result ^= result >> 3;
        }
        result
    }

    #[inline(never)]
    fn method_light_work(iterations: u32) -> u64 {
        let mut result: u64 = 0;
        for i in 0..iterations {
            result = result.wrapping_add(black_box(i as u64));
        }
        result
    }
}

struct OtherWorker;

impl OtherWorker {
    #[inline(never)]
    #[hotpath::measure(impl_type = "OtherWorker")]
    fn method_heavy_work(iterations: u32) -> u64 {
        let mut result: u64 = 1;
        for i in 0..iterations {
            result = result.wrapping_mul(black_box(i as u64).wrapping_add(7));
        }
        result
    }
}

#[hotpath::main]
fn main() {
    let mut total: u64 = 0;
    for _ in 0..2000 {
        total = total.wrapping_add(free_heavy_work(50_000));
        total = total.wrapping_add(Worker::method_heavy_work(50_000));
        total = total.wrapping_add(Worker::method_light_work(10_000));
        total = total.wrapping_add(OtherWorker::method_heavy_work(50_000));
    }
    black_box(total);
}

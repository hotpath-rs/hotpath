//! Validates CPU attribution for free functions, `measure_all` impls,
//! `measure(impl_type = ...)` on an inherent impl function, and bare `measure`
//! on an inherent method and a trait impl method. A bare `measure` on a
//! generic type's method is called too: its symbols carry the concrete type
//! arguments, so it gets no CPU samples.
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

impl OtherWorker {
    #[inline(never)]
    #[hotpath::measure]
    fn receiver_heavy_work(&self, iterations: u32) -> u64 {
        let mut result: u64 = 1;
        for i in 0..iterations {
            result = result.wrapping_mul(black_box(i as u64).wrapping_add(11));
        }
        result
    }
}

trait Heavy {
    fn trait_heavy_work(&self, iterations: u32) -> u64;
}

impl Heavy for OtherWorker {
    #[inline(never)]
    #[hotpath::measure]
    fn trait_heavy_work(&self, iterations: u32) -> u64 {
        let mut result: u64 = 1;
        for i in 0..iterations {
            result = result.wrapping_mul(black_box(i as u64).wrapping_add(13));
        }
        result
    }
}

struct Generic<T>(T);

impl<T: Copy + Into<u64>> Generic<T> {
    #[inline(never)]
    #[hotpath::measure]
    fn generic_heavy_work(&self, iterations: u32) -> u64 {
        let seed = self.0.into();
        let mut result: u64 = 1;
        for i in 0..iterations {
            result = result.wrapping_mul(black_box(i as u64).wrapping_add(seed));
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
        total = total.wrapping_add(OtherWorker.receiver_heavy_work(50_000));
        total = total.wrapping_add(OtherWorker.trait_heavy_work(50_000));
        total = total.wrapping_add(Generic(3u8).generic_heavy_work(25_000));
        total = total.wrapping_add(Generic(5u32).generic_heavy_work(25_000));
    }
    black_box(total);
}

//! Validates CPU attribution for free functions, `measure_all` impls, bare
//! `measure` on inherent associated functions and methods, trait impls (two
//! traits with one method name on one type), a generic impl called with two
//! types, a trait default method next to an unmeasured override, and a blanket
//! impl.
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
    #[hotpath::measure]
    fn method_heavy_work(iterations: u32) -> u64 {
        let mut result: u64 = 1;
        for i in 0..iterations {
            result = result.wrapping_mul(black_box(i as u64).wrapping_add(7));
        }
        result
    }

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
    fn work(&self, iterations: u32) -> u64;
}

trait Light {
    fn work(&self, iterations: u32) -> u64;
}

impl Heavy for OtherWorker {
    #[inline(never)]
    #[hotpath::measure]
    fn work(&self, iterations: u32) -> u64 {
        let mut result: u64 = 1;
        for i in 0..iterations {
            result = result.wrapping_mul(black_box(i as u64).wrapping_add(13));
        }
        result
    }
}

impl Light for OtherWorker {
    #[inline(never)]
    #[hotpath::measure]
    fn work(&self, iterations: u32) -> u64 {
        let mut result: u64 = 1;
        for i in 0..iterations {
            result = result.wrapping_mul(black_box(i as u64).wrapping_add(17));
        }
        result
    }
}

struct Generic<T>(T);

impl<T: Copy + Into<u64>> Generic<T> {
    #[inline(never)]
    #[hotpath::measure]
    fn generic_work(&self, iterations: u32) -> u64 {
        let seed = self.0.into();
        let mut result: u64 = 1;
        for i in 0..iterations {
            result = result.wrapping_mul(black_box(i as u64).wrapping_add(seed));
        }
        result
    }
}

trait Defaulted {
    #[inline(never)]
    #[hotpath::measure]
    fn default_work(&self, iterations: u32) -> u64 {
        let mut result: u64 = 1;
        for i in 0..iterations {
            result = result.wrapping_mul(black_box(i as u64).wrapping_add(19));
        }
        result
    }
}

impl Defaulted for Worker {}

// Unmeasured override: its samples must not go to the default method.
struct Overrides;

impl Defaulted for Overrides {
    #[inline(never)]
    fn default_work(&self, iterations: u32) -> u64 {
        let mut result: u64 = 1;
        for i in 0..iterations {
            result = result.wrapping_mul(black_box(i as u64).wrapping_add(23));
        }
        result
    }
}

trait Blanket {
    fn blanket_work(&self, iterations: u32) -> u64;
}

impl<T> Blanket for T {
    #[inline(never)]
    #[hotpath::measure]
    fn blanket_work(&self, iterations: u32) -> u64 {
        let mut result: u64 = 1;
        for i in 0..iterations {
            result = result.wrapping_mul(black_box(i as u64).wrapping_add(29));
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
        total = total.wrapping_add(Heavy::work(&OtherWorker, 50_000));
        total = total.wrapping_add(Light::work(&OtherWorker, 50_000));
        total = total.wrapping_add(Generic(3u8).generic_work(25_000));
        total = total.wrapping_add(Generic(5u32).generic_work(25_000));
        total = total.wrapping_add(Worker.default_work(50_000));
        total = total.wrapping_add(Overrides.default_work(50_000));
        total = total.wrapping_add(1u8.blanket_work(25_000));
        total = total.wrapping_add("x".blanket_work(25_000));
    }
    black_box(total);
}

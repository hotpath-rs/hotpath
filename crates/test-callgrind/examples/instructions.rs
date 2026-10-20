//! Per-function instruction counts under Callgrind for every shape the
//! measure macros wrap: free, generic, inherent, trait and recursive
//! functions, `measure_all` modules, `label` and `log = true` variants,
//! functions a `macro_rules!` generates and `#[track_caller]` ones (both
//! excluded), and async functions polled
//! on current-thread, multi-thread and spawned tokio tasks. Each measured
//! body runs `spin`, so expected counts are multiples of one `spin(N)`.
//!
//! Run with:
//!   CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER="valgrind -q --tool=callgrind" cargo run --release -p test-callgrind --example instructions --features hotpath,hotpath-callgrind

use std::hint::black_box;

const N: u64 = 200_000;

#[inline(never)]
fn spin(n: u64) -> u64 {
    let mut acc = 0u64;
    for i in 0..n {
        acc = acc.wrapping_mul(31).wrapping_add(black_box(i));
    }
    acc
}

#[hotpath::measure]
fn free_fn() -> u64 {
    spin(N)
}

#[hotpath::measure]
fn generic_fn<T: Into<u64>>(n: T) -> u64 {
    spin(n.into())
}

#[hotpath::measure]
fn recursive(depth: u32) -> u64 {
    let own = spin(N);
    if depth == 0 {
        own
    } else {
        own.wrapping_add(recursive(depth - 1))
    }
}

#[hotpath::measure(label = "labeled")]
fn labeled_fn() -> u64 {
    spin(N)
}

#[hotpath::measure(log = true)]
fn logged_fn() -> u64 {
    spin(N)
}

struct Worker {
    values: Vec<u64>,
}

#[hotpath::measure_all]
impl Worker {
    fn inherent(&self) -> u64 {
        spin(N)
    }

    // Returning a borrow of `self` through the marker closure.
    fn first_mut(&mut self) -> &mut u64 {
        spin(N);
        &mut self.values[0]
    }
}

trait Job {
    fn run(&self) -> u64;
}

impl Job for Worker {
    #[hotpath::measure]
    fn run(&self) -> u64 {
        spin(N)
    }
}

#[hotpath::measure]
async fn async_fn() -> u64 {
    let mut acc = 0;
    for _ in 0..4 {
        acc += spin(N);
        tokio::task::yield_now().await;
    }
    acc
}

#[hotpath::measure(future = true)]
async fn tracked_future() -> u64 {
    spin(N)
}

// Two measured fns expanded from one source position share a marker, so
// neither gets an instruction count.
macro_rules! measured_in_mod {
    ($module:ident) => {
        mod $module {
            #[hotpath::measure]
            pub fn generated() -> u64 {
                crate::spin(crate::N)
            }
        }
    };
}

measured_in_mod!(alpha);
measured_in_mod!(beta);

#[hotpath::measure_all]
mod nested {
    pub fn first() -> u64 {
        crate::spin(crate::N)
    }

    pub fn second() -> u64 {
        crate::spin(crate::N)
    }
}

// Keep their caller propagation, so they get no instruction count.
#[hotpath::measure]
#[track_caller]
fn caller_line() -> u32 {
    std::panic::Location::caller().line()
}

#[hotpath::measure]
#[cfg_attr(all(), track_caller)]
fn conditional_caller_line() -> u32 {
    std::panic::Location::caller().line()
}

#[hotpath::main]
fn main() {
    let mut total = 0u64;
    total = total.wrapping_add(free_fn());
    total = total.wrapping_add(free_fn());
    total = total.wrapping_add(generic_fn(N as u32));
    total = total.wrapping_add(generic_fn(N));
    total = total.wrapping_add(recursive(3));
    total = total.wrapping_add(labeled_fn());
    total = total.wrapping_add(logged_fn());
    total = total.wrapping_add(alpha::generated());
    total = total.wrapping_add(beta::generated());
    total = total.wrapping_add(nested::first());
    total = total.wrapping_add(nested::second());
    assert_eq!(caller_line(), line!(), "#[track_caller] must propagate");
    let (conditional_line, line) = (conditional_caller_line(), line!());
    assert_eq!(
        conditional_line, line,
        "cfg_attr track_caller must propagate"
    );

    let mut worker = Worker { values: vec![1] };
    total = total.wrapping_add(worker.inherent());
    total = total.wrapping_add(worker.run());
    *worker.first_mut() += 1;

    let current = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    total = total.wrapping_add(current.block_on(async_fn()));

    let multi = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .build()
        .unwrap();
    total = total.wrapping_add(multi.block_on(async {
        let spawned = tokio::spawn(async_fn());
        tracked_future().await + spawned.await.unwrap()
    }));

    black_box(total);
}

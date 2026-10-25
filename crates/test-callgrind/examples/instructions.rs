//! Per-function instruction counts under Callgrind for every shape the
//! measure macros wrap: free, generic, inherent, trait and recursive
//! functions, `measure_all` modules, `label` and `log = true` variants,
//! functions a `macro_rules!` generates and `#[track_caller]` ones (both
//! excluded), async functions polled on current-thread, multi-thread and
//! spawned tokio tasks, and `measure_block!` bodies, counted unless running
//! them as a closure would change what they do. Each measured body runs
//! `spin`, so expected counts are multiples of one `spin(N)`.
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

// The body's value is coerced to the declared return type (`&Vec<u64>` to
// `&[u64]`), which the marker closure must keep doing.
#[hotpath::measure_all]
impl Worker {
    fn values(&self) -> Option<(u64, &[u64])> {
        let first = *self.values.first()?;
        Some((first, &self.values))
    }
}

// Return types a closure cannot or need not name the same way: an
// `impl Trait` (kept inferred), `Self`, and an early `?` return.
#[hotpath::measure_all]
impl Worker {
    fn evens(&self) -> impl Iterator<Item = &u64> + '_ {
        self.values.iter().filter(|v| *v % 2 == 0)
    }

    fn cloned(&self) -> Self {
        Worker {
            values: self.values.clone(),
        }
    }

    fn parse_first(&self, input: &str) -> Result<u64, std::num::ParseIntError> {
        let parsed: u64 = input.parse()?;
        Ok(parsed + self.values[0])
    }
}

#[hotpath::measure(log = true)]
fn logged_slice(worker: &Worker) -> Option<(u64, &[u64])> {
    Some((worker.values.len() as u64, &worker.values))
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

// `measure_block!` bodies that run the same as a closure are counted.
#[hotpath::measure]
fn with_block() -> u64 {
    spin(N).wrapping_add(hotpath::measure_block!("nested_block", spin(N)))
}

fn loop_block() -> u64 {
    hotpath::measure_block!("loop_block", {
        let mut acc = 0u64;
        for i in 0.. {
            if i == 1 {
                break;
            }
            acc = acc.wrapping_add(spin(N));
        }
        acc
    })
}

fn labeled_block(skip: bool) -> u64 {
    hotpath::measure_block!("labeled_block", 'done: {
        if skip {
            break 'done 0;
        }
        let value = spin(N);
        assert_ne!(value, 1);
        value
    })
}

// Ones that would act on the closure instead of the enclosing fn or loop
// are measured but not counted.
fn try_block(input: &str) -> Result<u64, std::num::ParseIntError> {
    hotpath::measure_block!("try_block", {
        let parsed: u64 = input.parse()?;
        Ok(spin(N).wrapping_add(parsed))
    })
}

fn macro_try_block(input: &str) -> Result<String, std::num::ParseIntError> {
    Ok(hotpath::measure_block!(
        "macro_try_block",
        format!("{}", spin(N).wrapping_add(input.parse::<u64>()?))
    ))
}

fn return_block(skip: bool) -> u64 {
    hotpath::measure_block!("return_block", {
        if skip {
            return 0;
        }
        spin(N)
    })
}

macro_rules! bail_if {
    ($cond:expr) => {
        if $cond {
            return 0;
        }
    };
}

fn hidden_return_block(skip: bool) -> u64 {
    hotpath::measure_block!("hidden_return_block", {
        bail_if!(skip);
        spin(N)
    })
}

fn outer_break_block() -> u64 {
    let mut acc = 0u64;
    loop {
        hotpath::measure_block!("outer_break_block", {
            acc = acc.wrapping_add(spin(N));
            break;
        })
    }
    acc
}

async fn await_block() -> u64 {
    hotpath::measure_block!("await_block", {
        tokio::task::yield_now().await;
        spin(N)
    })
}

fn runtime_label_block() -> u64 {
    let label: &'static str = "runtime_block";
    hotpath::measure_block!(label, spin(N))
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
    total = total.wrapping_add(worker.values().map_or(0, |(_, v)| v.len() as u64));
    total = total.wrapping_add(logged_slice(&worker).map_or(0, |(n, _)| n));
    total = total.wrapping_add(worker.evens().count() as u64);
    total = total.wrapping_add(worker.cloned().values.len() as u64);
    total = total.wrapping_add(worker.parse_first("7").unwrap_or(0));

    for _ in 0..2 {
        total = total.wrapping_add(hotpath::measure_block!("plain_block", spin(N)));
    }
    total = total.wrapping_add(with_block());
    total = total.wrapping_add(loop_block());
    total = total.wrapping_add(labeled_block(false));
    // The body's value is coerced to the binding's type through the marker
    // closure: `&Vec<u64>` to `&[u64]`, and a borrow out of a capture.
    let values: &[u64] = hotpath::measure_block!("coerce_block", &worker.values);
    total = total.wrapping_add(values.len() as u64);
    let first: &mut u64 = hotpath::measure_block!("borrow_block", &mut worker.values[0]);
    *first += 1;
    total = total.wrapping_add(try_block("7").unwrap_or(0));
    total = total.wrapping_add(macro_try_block("7").map_or(0, |s| s.len() as u64));
    total = total.wrapping_add(return_block(false));
    total = total.wrapping_add(hidden_return_block(false));
    total = total.wrapping_add(outer_break_block());
    total = total.wrapping_add(runtime_label_block());

    let current = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    total = total.wrapping_add(current.block_on(async_fn()));
    total = total.wrapping_add(current.block_on(await_block()));

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

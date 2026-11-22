//! Per-call cost of `#[hotpath::measure]` for every function shape, so a change
//! to what the macro emits into the measured function itself (the code before
//! and around the guard) shows up as an instruction count delta. `benchmark_meta`
//! cannot see it: its `hotpath-meta` rows cover the profiler's own functions,
//! and the emitted prologue runs in the user's function, outside all of them.
//!
//! Each `drive_*` function is measured by `hotpath-meta` and calls one shape
//! `RUNS` times, so its inclusive count divided by `RUNS` is the full per-call
//! cost of that shape. `drive_raw*` call uninstrumented twins, as the baseline.
//! `drive_first_calls` makes the first call of fresh sites only, so it carries
//! the one-time per-site setup. Single-threaded and sleep-free, so call counts
//! are identical between runs. Changing `RUNS` invalidates comparability with
//! previously uploaded reports.
//!
//! Run with:
//!   cargo run --release -p test-all-features --example benchmark_measure --features hotpath,hotpath-alloc-meta

use std::future::Future;
use std::hint::black_box;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

#[cfg(feature = "hotpath-meta")]
use hotpath::hotpath_meta;

const RUNS: u64 = 100_000;

#[inline(never)]
fn raw_fn(v: u64) -> u64 {
    black_box(v).wrapping_mul(31)
}

#[inline(never)]
async fn raw_async(v: u64) -> u64 {
    black_box(v).wrapping_mul(31)
}

#[hotpath::measure(log = true)]
fn free_fn(v: u64) -> u64 {
    black_box(v).wrapping_mul(31)
}

#[hotpath::measure(label = "benchmark_measure_labeled")]
fn labeled_fn(v: u64) -> u64 {
    black_box(v).wrapping_mul(31)
}

struct Widget {
    seed: u64,
}

impl Widget {
    #[hotpath::measure]
    fn inherent(&self, v: u64) -> u64 {
        black_box(v ^ self.seed).wrapping_mul(31)
    }

    #[hotpath::measure]
    fn associated(v: u64) -> u64 {
        black_box(v).wrapping_mul(31)
    }

    #[hotpath::measure]
    async fn inherent_async(&self, v: u64) -> u64 {
        black_box(v ^ self.seed).wrapping_mul(31)
    }
}

struct Gadget {
    seed: u64,
}

#[hotpath::measure_all]
impl Gadget {
    fn via_measure_all(&self, v: u64) -> u64 {
        black_box(v ^ self.seed).wrapping_mul(31)
    }
}

trait Render {
    fn render(&self, v: u64) -> u64;
}

impl Render for Widget {
    #[hotpath::measure]
    fn render(&self, v: u64) -> u64 {
        black_box(v ^ self.seed).wrapping_mul(31)
    }
}

trait Defaulted {
    fn seed(&self) -> u64;

    #[hotpath::measure]
    fn defaulted(&self, v: u64) -> u64 {
        black_box(v ^ self.seed()).wrapping_mul(31)
    }
}

impl Defaulted for Widget {
    fn seed(&self) -> u64 {
        self.seed
    }
}

trait Blanket {
    fn blanket(&self, v: u64) -> u64;
}

impl<T> Blanket for T {
    #[hotpath::measure]
    fn blanket(&self, v: u64) -> u64 {
        black_box(v).wrapping_mul(31)
    }
}

struct Holder<T> {
    value: T,
}

impl<T: Copy + Into<u64>> Holder<T> {
    #[hotpath::measure]
    fn generic(&self, v: u64) -> u64 {
        black_box(v ^ self.value.into()).wrapping_mul(31)
    }
}

// Sites called once, from `drive_first_calls` only.
#[hotpath::measure]
fn first_free_fn(v: u64) -> u64 {
    black_box(v).wrapping_mul(31)
}

impl Widget {
    #[hotpath::measure]
    fn first_inherent(&self, v: u64) -> u64 {
        black_box(v ^ self.seed).wrapping_mul(31)
    }
}

trait FirstRender {
    fn first_render(&self, v: u64) -> u64;
}

impl FirstRender for Widget {
    #[hotpath::measure]
    fn first_render(&self, v: u64) -> u64 {
        black_box(v ^ self.seed).wrapping_mul(31)
    }
}

/// Polls a future that completes without waiting, as every async shape here does.
fn ready<F: Future>(fut: F) -> F::Output {
    let mut fut = pin!(fut);
    let mut cx = Context::from_waker(Waker::noop());
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(v) => v,
        Poll::Pending => unreachable!("benchmark futures complete on the first poll"),
    }
}

macro_rules! drivers {
    ($($name:ident => |$v:ident| $call:expr;)*) => {
        $(
            #[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure)]
            #[inline(never)]
            fn $name() -> u64 {
                let mut acc = 0u64;
                for $v in 0..RUNS {
                    acc = acc.wrapping_add($call);
                }
                acc
            }
        )*

        const DRIVERS: usize = [$(stringify!($name)),*].len();

        fn run_drivers() -> u64 {
            0u64 $(.wrapping_add($name()))*
        }
    };
}

const WIDGET: Widget = Widget { seed: 7 };
const GADGET: Gadget = Gadget { seed: 7 };
const HOLDER: Holder<u32> = Holder { value: 7 };

drivers! {
    drive_raw => |v| raw_fn(v);
    drive_raw_async => |v| ready(raw_async(v));
    drive_free_fn => |v| free_fn(v);
    drive_labeled => |v| labeled_fn(v);
    drive_inherent => |v| WIDGET.inherent(v);
    drive_associated => |v| Widget::associated(v);
    drive_inherent_async => |v| ready(WIDGET.inherent_async(v));
    drive_measure_all => |v| GADGET.via_measure_all(v);
    drive_trait_impl => |v| WIDGET.render(v);
    drive_trait_default => |v| WIDGET.defaulted(v);
    drive_blanket => |v| 7u8.blanket(v);
    drive_generic => |v| HOLDER.generic(v);
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure)]
#[inline(never)]
fn drive_first_calls() -> u64 {
    first_free_fn(1)
        .wrapping_add(WIDGET.first_inherent(2))
        .wrapping_add(WIDGET.first_render(3))
}

#[hotpath::main]
fn main() {
    let start = std::time::Instant::now();
    let total = drive_first_calls().wrapping_add(run_drivers());
    black_box(total);
    println!(
        "benchmark_measure: {DRIVERS} drivers x {RUNS} calls in {:?}",
        start.elapsed()
    );
}

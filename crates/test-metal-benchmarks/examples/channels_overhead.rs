//! Instrumented channel traffic: each op sends a value through a
//! `hotpath::channel!`-wrapped `std::sync::mpsc` channel, does fixed integer
//! work and receives it. The measured `phase` runs `OPS` ops, `REPS` times,
//! with no warm-up. Every phase must return the same result.
//!
//! Run with:
//!   cargo run --release -p test-metal-benchmarks --example channels_overhead --features hotpath

use std::hint::black_box;
use std::sync::mpsc::channel;

const OPS: u64 = 500_000;
const REPS: usize = 10;
const WORK_ROUNDS: u32 = 64;

/// Fixed work: `WORK_ROUNDS` dependent xorshift64 rounds. Unlike an affine
/// (multiply-add) step, these do not fold into one operation.
#[inline(never)]
fn work(x: u64) -> u64 {
    let mut v = black_box(x) | 1;
    for _ in 0..WORK_ROUNDS {
        v ^= v << 13;
        v ^= v >> 7;
        v ^= v << 17;
    }
    v
}

/// `OPS` x (send, fixed work, recv). Takes closures because the instrumented
/// channel has its own `Sender`/`Receiver` types when `hotpath` is enabled.
#[hotpath::measure]
fn phase(send: &dyn Fn(u64), recv: &dyn Fn() -> u64) -> u64 {
    let mut acc = 0;
    for i in 0..OPS {
        send(i);
        acc ^= work(i);
        acc ^= recv();
    }
    black_box(acc)
}

#[hotpath::main(percentiles = [50, 95])]
fn main() {
    let (tx, rx) = hotpath::channel!(channel::<u64>(), label = "instrumented");
    let send = |v: u64| tx.send(v).expect("send");
    let recv = || rx.recv().expect("recv");

    let expected = phase(&send, &recv);
    for _ in 1..REPS {
        assert_eq!(phase(&send, &recv), expected);
    }
}

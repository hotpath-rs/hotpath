//! `outer` first-calls measured callees, whose location registration grows
//! the registry inside its scope. Only the allocations made by the measured
//! code itself may be reported: 50 B per `outer` call, 32 B per `callee_a`
//! call and nothing for the other callees.
//!
//! Run with:
//!   cargo run -p test-alloc --example first_call_registration --features hotpath-alloc

#[hotpath::measure]
fn callee_a(x: u64) -> u64 {
    let buf = vec![0u8; 32];
    std::hint::black_box(&buf);
    std::hint::black_box(x + 1)
}

#[hotpath::measure]
fn callee_b(x: u64) -> u64 {
    std::hint::black_box(x + 2)
}

#[hotpath::measure]
fn callee_c(x: u64) -> u64 {
    std::hint::black_box(x + 3)
}

#[hotpath::measure]
fn callee_d(x: u64) -> u64 {
    std::hint::black_box(x + 4)
}

#[hotpath::measure]
fn outer(x: u64) -> u64 {
    let buf = vec![0u8; 50];
    std::hint::black_box(&buf);
    callee_d(callee_c(callee_b(callee_a(x))))
}

#[hotpath::main]
fn main() {
    let mut acc = 0;
    for _ in 0..10 {
        acc = outer(acc);
    }
    std::hint::black_box(acc);
}

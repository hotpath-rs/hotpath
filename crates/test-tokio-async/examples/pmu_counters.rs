//! Run with:
//!   cargo run -p test-tokio-async --example pmu_counters --features hotpath,hotpath-pmu

#[hotpath::measure]
fn checksum(iterations: u64) -> u64 {
    let mut x = 0u64;
    for i in 0..iterations {
        x = std::hint::black_box(x.wrapping_mul(31).wrapping_add(i));
    }
    x
}

fn main() {
    let _guard = hotpath::HotpathGuardBuilder::new("pmu_counters")
        .sections(vec![
            hotpath::Section::FunctionsTiming,
            hotpath::Section::Threads,
        ])
        .build();

    let worker = std::thread::spawn(|| checksum(20_000_000));
    let local = checksum(20_000_000);
    println!("checksum: {}", local ^ worker.join().unwrap());
}

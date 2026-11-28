//! The `brrr-criterion-hetzner` hotpath-cloud benchmark: brrr, Jon Gjengset's
//! One Billion Row Challenge solution (https://github.com/jonhoo/brrr, written
//! on stream: https://youtu.be/tCY7p6dVAGE), as a Criterion benchmark profiled
//! by `hotpath-meta`. Worker threads each aggregate one chunk of the input and
//! send their maps over a channel once, so it is parallel with little
//! cross-thread communication, between the single-threaded `fib` and the
//! lockstep `drain` benchmarks. Only the cold functions (`run`, `one` per
//! worker, `print`) are instrumented.
//!
//! The input is generated in memory from a fixed seed, in the challenge's
//! `<station>;<temperature>` format, with synthetic station names.
//!
//! Run with:
//!   crates/test-brrr-criterion/fetch.sh
//!   cargo +nightly bench --manifest-path crates/test-brrr-criterion/Cargo.toml --bench brrr
//!   HOTPATH_META_CRITERION_ITERATIONS=5 cargo +nightly bench --manifest-path crates/test-brrr-criterion/Cargo.toml --bench brrr --features hotpath-alloc-meta -- --test --exact brrr/rows

// Upstream's remaining unstable features; the others it declares are stable now.
#![feature(portable_simd)]
#![feature(hasher_prefixfree_extras)]

// Written by `fetch.sh`; the file-reading `mmap` helpers stay unused.
#[allow(dead_code)]
#[path = "../upstream/brrr.rs"]
mod brrr;

use std::collections::HashSet;

use hotpath_meta::criterion::{Criterion, criterion_group, criterion_main};

/// Sized for about one second per call on the Hetzner runner. Changing the
/// input constants breaks comparability with previously uploaded reports.
const ROWS: usize = 100_000_000;
/// As many stations as the challenge's sample data.
const STATIONS: usize = 413;
const SEED: u64 = 0x1b2c_0000_0000_0001;

/// SplitMix64: deterministic across platforms and toolchains.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// Distinct names of 3 to 20 ASCII letters, so about a quarter overflow
/// brrr's 16-byte inline key and take its heap path.
fn stations(rng: &mut Rng) -> Vec<(Vec<u8>, i32)> {
    let mut seen = HashSet::new();
    let mut stations = Vec::with_capacity(STATIONS);
    while stations.len() < STATIONS {
        let len = 3 + rng.below(18) as usize;
        let mut name = vec![b'A' + rng.below(26) as u8];
        name.extend((1..len).map(|_| b'a' + rng.below(26) as u8));
        if seen.insert(name.clone()) {
            // Mean temperature in tenths of a degree, -10.0 to 30.0.
            let mean = rng.below(401) as i32 - 100;
            stations.push((name, mean));
        }
    }
    stations
}

fn generate() -> Vec<u8> {
    let mut rng = Rng(SEED);
    let stations = stations(&mut rng);
    let mut data = Vec::with_capacity(ROWS * 16);
    for _ in 0..ROWS {
        let (name, mean) = &stations[rng.below(STATIONS as u64) as usize];
        // Roughly bell-shaped spread of up to +-20.0 degrees around the mean.
        let spread: i32 = (0..4).map(|_| rng.below(101) as i32 - 50).sum();
        let t = (mean + spread).clamp(-999, 999);
        data.extend_from_slice(name);
        data.push(b';');
        if t < 0 {
            data.push(b'-');
        }
        let t = t.unsigned_abs();
        if t >= 100 {
            data.push(b'0' + (t / 100) as u8);
        }
        data.push(b'0' + (t / 10 % 10) as u8);
        data.push(b'.');
        data.push(b'0' + (t % 10) as u8);
        data.push(b'\n');
    }
    data
}

fn bench_brrr(c: &mut Criterion) {
    let data = generate();
    let mut group = c.benchmark_group("brrr");
    group.sample_size(10);
    group.bench_function("rows", |b| {
        b.iter(|| {
            let out = brrr::run(&data);
            // Every station appears once as `name=min/mean/max`.
            assert_eq!(out.iter().filter(|&&b| b == b'=').count(), STATIONS);
            out
        })
    });
    group.finish();
}

criterion_group!(benches, bench_brrr);
criterion_main!(benches);

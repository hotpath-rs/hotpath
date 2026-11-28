//! Writes the deterministic One Billion Row Challenge dataset of the
//! bare-metal benchmarks. `data/measurements-100M.sha256` is the hash of the
//! 100M-row file with the default seed.
//!
//! Run with:
//!   cargo run --release -p test-metal-benchmarks --example generate_1brc -- <output.txt> [rows]

use std::fs::File;
use std::io::{BufWriter, Write};

use test_metal_benchmarks::generate::{generate, SEED};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .expect("usage: generate_1brc <output.txt> [rows]");
    let rows: u64 = args
        .next()
        .map_or(100_000_000, |r| r.parse().expect("rows"));

    let mut out = BufWriter::with_capacity(1 << 20, File::create(&path).expect("create output"));
    generate(rows, SEED, &mut out).expect("write rows");
    out.flush().expect("flush output");
}

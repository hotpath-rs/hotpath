//! The `instructions` cloud benchmark (`.github/workflows/cloud-instructions-benchmark.yml`):
//! a small deterministic pipeline - generate records, render and parse them as
//! CSV, sort, aggregate, checksum, plus an async batch stage - whose
//! per-function instruction counts are exact under Callgrind, so a change to
//! any stage shows up as an exact diff. Hashing uses a fixed hasher, so no
//! run-to-run noise.
//!
//! Run with:
//!   CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER="valgrind -q --tool=callgrind" cargo run --release -p test-callgrind --example benchmark --features hotpath,hotpath-callgrind

use std::collections::HashMap;
use std::fmt::Write;
use std::hash::{BuildHasherDefault, DefaultHasher};

const RECORDS: usize = 20_000;
const KEYS: u64 = 64;
const BATCHES: usize = 8;

type FixedMap<K, V> = HashMap<K, V, BuildHasherDefault<DefaultHasher>>;

struct Record {
    id: u64,
    key: String,
    value: u64,
}

#[hotpath::measure]
fn generate_records(count: usize) -> Vec<Record> {
    let mut state = 0x2545_f491_4f6c_dd1du64;
    (0..count as u64)
        .map(|id| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            Record {
                id,
                key: format!("key-{}", state % KEYS),
                value: state % 10_000,
            }
        })
        .collect()
}

#[hotpath::measure]
fn render_csv(records: &[Record]) -> String {
    let mut out = String::with_capacity(records.len() * 24);
    for r in records {
        let _ = writeln!(out, "{},{},{}", r.id, r.key, r.value);
    }
    out
}

#[hotpath::measure]
fn parse_csv(input: &str) -> Vec<Record> {
    input
        .lines()
        .map(|line| {
            let mut fields = line.split(',');
            let id = fields.next().unwrap().parse().unwrap();
            let key = fields.next().unwrap().to_string();
            let value = fields.next().unwrap().parse().unwrap();
            Record { id, key, value }
        })
        .collect()
}

#[hotpath::measure]
fn sort_records(records: &mut [Record]) {
    records.sort_by(|a, b| a.key.cmp(&b.key).then(b.value.cmp(&a.value)));
}

#[hotpath::measure]
fn aggregate(records: &[Record]) -> FixedMap<String, (u64, u64)> {
    let mut totals: FixedMap<String, (u64, u64)> = FixedMap::default();
    for r in records {
        let entry = totals.entry(r.key.clone()).or_insert((0, 0));
        entry.0 += 1;
        entry.1 += r.value;
    }
    totals
}

#[hotpath::measure]
fn checksum(records: &[Record]) -> u64 {
    records.iter().fold(0xcbf2_9ce4_8422_2325, |hash, r| {
        (hash ^ r.value ^ r.id).wrapping_mul(0x0100_0000_01b3)
    })
}

#[hotpath::measure]
async fn process_batch(batch: &[Record]) -> u64 {
    let mut acc = 0u64;
    for chunk in batch.chunks(256) {
        acc = acc.wrapping_add(chunk.iter().map(|r| r.value * r.value).sum::<u64>());
        tokio::task::yield_now().await;
    }
    acc
}

#[hotpath::main]
fn main() {
    let records = generate_records(RECORDS);
    let csv = render_csv(&records);
    let mut parsed = parse_csv(&csv);
    sort_records(&mut parsed);
    let groups = aggregate(&parsed);
    let sum = checksum(&parsed);

    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let batches: u64 = runtime.block_on(async {
        let mut total = 0u64;
        for batch in parsed.chunks(parsed.len() / BATCHES) {
            total = total.wrapping_add(process_batch(batch).await);
        }
        total
    });

    std::hint::black_box((groups.len(), sum, batches));
}

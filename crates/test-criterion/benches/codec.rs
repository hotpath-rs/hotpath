//! Minimal Criterion benchmark using `hotpath::criterion`.
//!
//! Run with:
//!   cargo bench -p test-criterion --bench codec
//!   HOTPATH_CRITERION_ITERATIONS=1000 cargo bench -p test-criterion --bench codec --features hotpath-alloc -- --test --exact codec/encode
//!
//! On Criterion 0.5, 0.6 and 0.7 (`crates/test-criterion-0-5`, `-0-6` and `-0-7`
//! build this same file):
//!   cargo bench -p test-criterion-0-5 --bench codec
//!   HOTPATH_CRITERION_ITERATIONS=1000 cargo bench -p test-criterion-0-5 --bench codec --features hotpath-alloc -- --test --exact codec/encode
//!   cargo bench -p test-criterion-0-6 --bench codec
//!   HOTPATH_CRITERION_ITERATIONS=1000 cargo bench -p test-criterion-0-6 --bench codec --features hotpath-alloc -- --test --exact codec/encode
//!   cargo bench -p test-criterion-0-7 --bench codec
//!   HOTPATH_CRITERION_ITERATIONS=1000 cargo bench -p test-criterion-0-7 --bench codec --features hotpath-alloc -- --test --exact codec/encode

use std::hint::black_box;

use hotpath::criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};

#[hotpath::measure]
fn encode(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

#[hotpath::measure]
fn decode(bytes: &[u8]) -> Vec<u32> {
    let (chunks, _) = bytes.as_chunks::<4>();
    chunks.iter().map(|c| u32::from_le_bytes(*c)).collect()
}

fn bench_codec(c: &mut Criterion) {
    let values: Vec<u32> = (0..256).collect();
    let mut group = c.benchmark_group("codec");
    group.bench_function("encode", |b| b.iter(|| encode(black_box(&values))));
    group.bench_function("decode", |b| {
        b.iter_batched(
            || encode(&values),
            |bytes| decode(black_box(&bytes)),
            BatchSize::SmallInput,
        )
    });
    for len in [16usize, 1024] {
        let input: Vec<u32> = (0..len as u32).collect();
        group.bench_with_input(BenchmarkId::new("roundtrip", len), &input, |b, input| {
            b.iter(|| decode(&encode(black_box(input))))
        });
    }
    group.finish();
}

criterion_group!(benches, bench_codec);
criterion_main!(benches);

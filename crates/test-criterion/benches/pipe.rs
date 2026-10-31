//! Async Criterion benchmark using `hotpath::criterion`: values round-trip
//! through an in-memory Tokio pipe smaller than the payload, so the writer and
//! reader take turns across `.await` points.
//!
//! Run with:
//!   cargo bench -p test-criterion --bench pipe
//!   HOTPATH_CRITERION_ITERATIONS=1000 cargo bench -p test-criterion --bench pipe --features hotpath-alloc -- --test --exact pipe/roundtrip/4096
//!
//! On Criterion 0.5 (`crates/test-criterion-0-5` builds this same file):
//!   cargo bench -p test-criterion-0-5 --bench pipe
//!   HOTPATH_CRITERION_ITERATIONS=1000 cargo bench -p test-criterion-0-5 --bench pipe --features hotpath-alloc -- --test --exact pipe/roundtrip/4096

use std::hint::black_box;

use hotpath::criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const PIPE_CAPACITY: usize = 1024;

#[hotpath::measure]
fn encode(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

#[hotpath::measure]
fn decode(bytes: &[u8]) -> Vec<u32> {
    let (chunks, _) = bytes.as_chunks::<4>();
    chunks.iter().map(|c| u32::from_le_bytes(*c)).collect()
}

#[hotpath::measure]
async fn roundtrip(values: &[u32]) -> Vec<u32> {
    let (mut tx, mut rx) = tokio::io::duplex(PIPE_CAPACITY);
    let bytes = encode(values);
    let write = async move {
        tx.write_all(&bytes).await.unwrap();
        tx.shutdown().await.unwrap();
    };
    let read = async {
        let mut received = Vec::with_capacity(values.len() * 4);
        rx.read_to_end(&mut received).await.unwrap();
        received
    };
    let ((), received) = tokio::join!(write, read);
    decode(&received)
}

fn bench_pipe(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let mut group = c.benchmark_group("pipe");
    for len in [256usize, 4096] {
        let values: Vec<u32> = (0..len as u32).collect();
        group.bench_with_input(BenchmarkId::new("roundtrip", len), &values, |b, values| {
            b.to_async(&rt).iter(|| roundtrip(black_box(values)))
        });
    }
    group.finish();
}

criterion_group!(benches, bench_pipe);
criterion_main!(benches);

//! Callgrind stability POC: per-function inclusive instruction counts
//! across repeated runs and runners, for sync, async and concurrent
//! workloads.
//!
//! Run with:
//!   cargo build --release --manifest-path crates/callgrind-poc/Cargo.toml
//!   HOTPATH_OUTPUT_FORMAT=none HOTPATH_REPORT=functions-timing valgrind --tool=callgrind crates/callgrind-poc/target/release/callgrind-poc

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::hash::{BuildHasherDefault, DefaultHasher};
use std::hint::black_box;
use std::time::Duration;

/// Simulates what hotpath's future wrapper would need under Callgrind: a
/// poll that is never inlined, so every poll of the body is attributed to
/// one symbol named after the measured function's future type.
struct NoInline<F>(F);

impl<F: Future> Future for NoInline<F> {
    type Output = F::Output;

    #[inline(never)]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<F::Output> {
        // SAFETY: the inner future is never moved out of the pinned wrapper.
        unsafe { self.map_unchecked_mut(|s| &mut s.0) }.poll(cx)
    }
}

#[inline(never)]
fn spin(n: u64) -> u64 {
    let mut x = 0u64;
    for i in 0..n {
        x = black_box(x.wrapping_mul(31).wrapping_add(i));
    }
    x
}

#[hotpath::measure]
#[inline(never)]
fn sync_checksum(n: u64) -> u64 {
    spin(n)
}

/// std HashMap with RandomState: random SipHash keys per run.
#[hotpath::measure]
#[inline(never)]
fn sync_hashmap_random(n: u64) -> u64 {
    let mut m = HashMap::new();
    for i in 0..n {
        m.insert(i, i * 2);
    }
    (0..n).map(|i| m[&i]).sum()
}

/// Same map with fixed SipHash keys.
#[hotpath::measure]
#[inline(never)]
fn sync_hashmap_fixed(n: u64) -> u64 {
    let mut m: HashMap<u64, u64, BuildHasherDefault<DefaultHasher>> = HashMap::default();
    for i in 0..n {
        m.insert(i, i * 2);
    }
    (0..n).map(|i| m[&i]).sum()
}

/// Allocation-heavy: Vec<String> building.
#[hotpath::measure]
#[inline(never)]
fn sync_alloc(n: u64) -> usize {
    let v: Vec<String> = (0..n).map(|i| format!("item-{i}")).collect();
    v.iter().map(|s| s.len()).sum()
}

/// memchr picks SSE2/AVX2 code at runtime from the CPU features Valgrind
/// reports, so this one may differ between runner CPU models.
#[hotpath::measure]
#[inline(never)]
fn sync_memchr(n: usize) -> usize {
    let mut haystack = vec![b'a'; n];
    haystack[n - 1] = b'z';
    let needle = b"aaaz";
    let mut found = 0;
    for _ in 0..20 {
        found += memchr::memmem::find(black_box(&haystack), needle).unwrap_or(0);
        found += memchr::memchr(b'z', black_box(&haystack)).unwrap_or(0);
    }
    found
}

/// Compute split by yield points: poll count is fixed (10 yields).
#[hotpath::measure]
async fn async_yield_compute(n: u64) -> u64 {
    NoInline(async move {
        let mut acc = 0;
        for _ in 0..10 {
            acc ^= spin(n / 10);
            tokio::task::yield_now().await;
        }
        acc
    })
    .await
}

/// Compute between timer waits: polls depend on timer wakeups.
#[hotpath::measure]
async fn async_sleep_compute(n: u64) -> u64 {
    NoInline(async move {
        let mut acc = 0;
        for _ in 0..5 {
            tokio::time::sleep(Duration::from_millis(2)).await;
            acc ^= spin(n / 5);
        }
        acc
    })
    .await
}

/// Multi-threaded: 4 producer tasks feed a bounded channel, consumer sums.
#[hotpath::measure]
async fn async_channel_pipeline(n: u64) -> u64 {
    NoInline(async move {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<u64>(16);
        for p in 0..4u64 {
            let tx = tx.clone();
            tokio::spawn(async move {
                for i in 0..n / 4 {
                    tx.send(spin(50) ^ (p * i)).await.unwrap();
                }
            });
        }
        drop(tx);
        let mut sum = 0u64;
        while let Some(v) = rx.recv().await {
            sum = sum.wrapping_add(v);
        }
        sum
    })
    .await
}

fn main() {
    #[cfg(target_arch = "x86_64")]
    println!(
        "cpu-features sse4.2={} avx2={} avx512f={}",
        std::is_x86_feature_detected!("sse4.2"),
        std::is_x86_feature_detected!("avx2"),
        std::is_x86_feature_detected!("avx512f"),
    );

    let _guard = hotpath::HotpathGuardBuilder::new("callgrind-poc").build();

    println!("{}", sync_checksum(2_000_000));
    println!("{}", sync_hashmap_random(20_000));
    println!("{}", sync_hashmap_fixed(20_000));
    println!("{}", sync_alloc(20_000));
    println!("{}", sync_memchr(1_000_000));

    let current = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    println!("{}", current.block_on(async_yield_compute(2_000_000)));
    println!("{}", current.block_on(async_sleep_compute(2_000_000)));

    let multi = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();
    println!("{}", multi.block_on(async_yield_compute(2_000_000)));
    println!("{}", multi.block_on(async_channel_pipeline(20_000)));
}

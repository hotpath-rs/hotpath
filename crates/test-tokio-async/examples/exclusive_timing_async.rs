//! Async functions awaiting measured children one by one, concurrently, with
//! cancellation and through a spawned task, for checking exclusive
//! (`HOTPATH_TIME_EXCLUSIVE`) function timing. Every parent spins for 50ms of
//! its own time before awaiting its children.
//!
//! Run with:
//!   HOTPATH_TIME_EXCLUSIVE=true cargo run -p test-tokio-async --example exclusive_timing_async --features hotpath

use std::time::{Duration, Instant};

/// Spins instead of sleeping, so a parent's own time is not blurred by timer
/// overshoot on loaded CI runners.
fn busy(ms: u64) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(ms) {
        std::hint::spin_loop();
    }
}

async fn wait(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

/// No children: the whole wait is its own time.
#[hotpath::measure]
async fn io_wait() {
    wait(100).await;
}

#[hotpath::measure]
async fn seq_child() {
    wait(100).await;
}

#[hotpath::measure]
async fn seq_parent() {
    busy(50);
    seq_child().await;
}

#[hotpath::measure]
async fn join_short() {
    wait(100).await;
}

#[hotpath::measure]
async fn join_long() {
    wait(150).await;
}

/// Children overlap: they cover 150ms of the parent, not 250ms.
#[hotpath::measure]
async fn join_parent() {
    busy(50);
    tokio::join!(join_short(), join_long());
}

#[hotpath::measure]
async fn select_fast() {
    wait(100).await;
}

#[hotpath::measure]
async fn select_slow() {
    wait(500).await;
}

/// `select_slow` is cancelled when `select_fast` returns.
#[hotpath::measure]
async fn select_parent() {
    busy(50);
    tokio::select! {
        _ = select_fast() => {}
        _ = select_slow() => {}
    }
}

#[hotpath::measure]
async fn mixed_async() {
    wait(100).await;
}

#[hotpath::measure]
fn mixed_sync() {
    busy(30);
}

/// The sync child runs while the async child is in flight.
#[hotpath::measure]
async fn mixed_parent() {
    busy(50);
    tokio::join!(mixed_async(), async { mixed_sync() });
}

#[hotpath::measure]
async fn spawned_child() {
    wait(100).await;
}

/// The child runs in its own task, so it is not nested in the parent.
#[hotpath::measure]
async fn spawn_parent() {
    busy(50);
    tokio::spawn(spawned_child()).await.unwrap();
}

#[tokio::main(flavor = "current_thread")]
#[hotpath::main(format = "json")]
async fn main() {
    io_wait().await;
    seq_parent().await;
    join_parent().await;
    select_parent().await;
    mixed_parent().await;
    spawn_parent().await;
}

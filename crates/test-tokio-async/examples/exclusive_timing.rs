//! Nested sync, recursive and async functions with known own/nested time
//! splits, for comparing inclusive and exclusive (`HOTPATH_TIME_EXCLUSIVE`)
//! function timing.
//!
//! Run with:
//!   HOTPATH_TIME_EXCLUSIVE=true cargo run -p test-tokio-async --example exclusive_timing --features hotpath

use std::time::{Duration, Instant};

fn busy(ms: u64) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(ms) {
        std::hint::spin_loop();
    }
}

fn sleep(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

#[hotpath::measure]
fn sync_child() {
    sleep(100);
}

#[hotpath::measure]
fn sync_parent() {
    sleep(50);
    sync_child();
}

#[hotpath::measure]
fn recursive(depth: u32) {
    sleep(20);
    if depth > 1 {
        recursive(depth - 1);
    }
}

#[hotpath::measure]
fn focus_leaf() {
    sleep(60);
}

#[hotpath::measure]
fn focus_mid() {
    sleep(30);
    focus_leaf();
}

#[hotpath::measure]
fn focus_outer() {
    sleep(30);
    focus_mid();
}

#[hotpath::measure]
async fn async_child() {
    busy(100);
    tokio::task::yield_now().await;
}

#[hotpath::measure]
async fn async_parent() {
    busy(50);
    async_child().await;
}

#[hotpath::measure]
async fn async_sleeper() {
    tokio::time::sleep(Duration::from_millis(100)).await;
}

#[tokio::main(flavor = "current_thread")]
#[hotpath::main(format = "json")]
async fn main() {
    sync_parent();
    recursive(4);
    focus_outer();
    async_parent().await;
    async_sleeper().await;
}

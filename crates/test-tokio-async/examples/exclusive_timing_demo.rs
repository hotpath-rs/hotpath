//! Side-by-side demo of inclusive (default) and exclusive function timing.
//! Every function name states how long the function itself sleeps, so in
//! exclusive mode each row's total matches its name, while in inclusive mode
//! parents also carry their children's time.
//!
//! Call tree (own time):
//!   handle_request_own_100ms
//!     load_user_own_200ms
//!     render_page_own_50ms
//!       render_row_own_50ms (x3)
//!
//! Run with:
//!   cargo run -p test-tokio-async --example exclusive_timing_demo --features hotpath
//!   HOTPATH_TIME_EXCLUSIVE=true cargo run -p test-tokio-async --example exclusive_timing_demo --features hotpath

use std::time::Duration;

fn sleep(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

#[hotpath::measure]
fn render_row_own_50ms() {
    sleep(50);
}

#[hotpath::measure]
fn render_page_own_50ms() {
    sleep(50);
    for _ in 0..3 {
        render_row_own_50ms();
    }
}

#[hotpath::measure]
fn load_user_own_200ms() {
    sleep(200);
}

#[hotpath::measure]
fn handle_request_own_100ms() {
    sleep(100);
    load_user_own_200ms();
    render_page_own_50ms();
}

#[hotpath::main]
fn main() {
    handle_request_own_100ms();
}

//! Exercises location registration for every instrumented-object kind read by
//! `crates/hotpath/tests/locations.rs`.
//!
//! Run with:
//!   cargo run -p test-tokio-async --example locations --features hotpath

use std::time::Duration;

#[path = "../support/path_module.rs"]
mod path_module;

#[hotpath::measure]
fn plain_function() {
    std::thread::sleep(Duration::from_millis(1));
}

#[hotpath::measure(label = "custom_label_fn")]
fn labeled_function() {
    std::thread::sleep(Duration::from_millis(1));
}

struct Worker;

#[hotpath::measure_all]
impl Worker {
    fn run(&self) {
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[tokio::main(flavor = "current_thread")]
#[hotpath::main(format = "json-pretty")]
async fn main() {
    plain_function();
    path_module::path_module_function();
    labeled_function();
    Worker.run();

    hotpath::measure_block!("locations_block", {
        std::thread::sleep(Duration::from_millis(1));
    });

    for flag in [true, false] {
        hotpath::measure_block!(if flag { "block_a" } else { "block_b" }, {
            std::thread::sleep(Duration::from_millis(1));
        });
    }

    let lock = hotpath::mutex!(std::sync::Mutex::new(0u32));
    *lock.lock().unwrap() += 1;

    let (tx, rx) = hotpath::channel!(std::sync::mpsc::channel::<u32>());
    tx.send(1).unwrap();
    drop(tx);
    while rx.recv().is_ok() {}

    let value = hotpath::future!(async { 42 }).await;
    std::hint::black_box(value);
}

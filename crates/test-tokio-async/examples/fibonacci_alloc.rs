//! Run with:
//!   cargo run -p test-tokio-async --example fibonacci_alloc --features hotpath-alloc

#[hotpath::measure]
fn fibonacci(n: u64) -> u64 {
    let buffer = vec![0u8; 1024];
    std::hint::black_box(&buffer);

    match n {
        0 => 0,
        1 => 1,
        _ => fibonacci(n - 1) + fibonacci(n - 2),
    }
}

#[hotpath::main]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    for n in [5, 8, 10, 12] {
        let result = fibonacci(n);
        println!("fibonacci({}) = {}", n, result);
    }

    Ok(())
}

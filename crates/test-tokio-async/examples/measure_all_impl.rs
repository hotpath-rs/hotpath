//! Run with:
//!   cargo run -p test-tokio-async --example measure_all_impl --features hotpath

use std::time::Duration;

struct Calculator {
    value: u64,
}

#[hotpath::measure_all]
impl Calculator {
    fn new(value: u64) -> Self {
        let vec = vec![1, 2, 3];
        std::hint::black_box(&vec);
        drop(vec);
        Self { value }
    }

    #[hotpath::measure]
    fn add(&mut self, amount: u64) {
        let vec = vec![4, 5, 6];
        std::hint::black_box(&vec);
        drop(vec);
        self.value += amount;
        std::thread::sleep(Duration::from_nanos(amount));
    }

    fn multiply(&mut self, factor: u64) {
        let vec = vec![7, 8, 9];
        std::hint::black_box(&vec);
        drop(vec);
        self.value *= factor;
        std::thread::sleep(Duration::from_nanos(factor * 2));
    }

    async fn async_increment(&mut self, amount: u64) {
        let vec = vec![10, 11, 12];
        std::hint::black_box(&vec);
        drop(vec);
        self.value += amount;
        tokio::time::sleep(Duration::from_nanos(amount)).await;
    }

    async fn async_decrement(&mut self, amount: u64) {
        let vec = vec![13, 14, 15];
        std::hint::black_box(&vec);
        drop(vec);
        self.value = self.value.saturating_sub(amount);
        tokio::time::sleep(Duration::from_nanos(amount * 2)).await;
    }

    fn get_value(&self) -> u64 {
        self.value
    }
}

#[tokio::main(flavor = "current_thread")]
#[hotpath::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    for i in 1..=50 {
        let mut calc = Calculator::new(100);
        calc.add(i);
        calc.multiply(2);
        calc.async_increment(i * 2).await;
        calc.async_decrement(i).await;
        std::hint::black_box(calc.get_value());
    }

    Ok(())
}

//! Names `#[hotpath::measure]` reports: inferred from the function's path
//! without `impl_type`, or `module::Type::fn` with it. Run with `rev` to call
//! the generic method's instantiations in the opposite order; the names must
//! not change.
//!
//! Run with:
//!   cargo run -p test-tokio-async --example function_names --features hotpath
//!   cargo run -p test-tokio-async --example function_names --features hotpath -- rev
//!   cargo run -p test-tokio-async --example function_names --features hotpath-alloc

use std::fmt;
use std::hint::black_box;

pub struct Widget {
    value: u64,
}

impl Widget {
    #[hotpath::measure]
    fn by_ref(&self) -> u64 {
        black_box(self.value)
    }

    #[hotpath::measure]
    fn by_mut(&mut self) {
        self.value += 1;
    }

    #[hotpath::measure]
    fn boxed(self: Box<Self>) -> u64 {
        black_box(self.value)
    }

    #[hotpath::measure]
    fn new(value: u64) -> Self {
        Self { value }
    }

    #[hotpath::measure]
    async fn by_ref_async(&self) -> u64 {
        black_box(self.value)
    }

    #[hotpath::measure(label = "widget_labeled")]
    fn labeled(&self) -> u64 {
        black_box(self.value)
    }

    #[hotpath::measure(impl_type = "Widget")]
    fn explicit(&self) -> u64 {
        black_box(self.value)
    }

    #[hotpath::measure(impl_type = "Other")]
    fn overridden(&self) -> u64 {
        black_box(self.value)
    }
}

// An inherent impl outside the type's module.
mod elsewhere {
    impl crate::Widget {
        #[hotpath::measure]
        pub(crate) fn remote(&self) -> u64 {
            std::hint::black_box(self.value)
        }
    }
}

// Two traits with one method name on one type.
impl fmt::Display for Widget {
    #[hotpath::measure]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "display")
    }
}

impl fmt::Debug for Widget {
    #[hotpath::measure]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "debug")
    }
}

pub struct Worker<T> {
    value: T,
}

impl<T: Copy + Into<u64>> Worker<T> {
    #[hotpath::measure]
    fn run(&self) -> u64 {
        black_box(self.value.into())
    }
}

pub struct Borrowed<'a> {
    text: &'a str,
}

trait Render {
    fn render(self) -> usize;
}

impl Render for &Borrowed<'_> {
    #[hotpath::measure]
    fn render(self) -> usize {
        black_box(self.text.len())
    }
}

pub struct Gear;

#[hotpath::measure_all]
impl Gear {
    fn turn(&self) -> u64 {
        black_box(3)
    }
}

#[hotpath::measure_all]
impl Render for &Gear {
    fn render(self) -> usize {
        black_box(4)
    }
}

#[hotpath::measure]
fn free_fn() -> u64 {
    black_box(5)
}

pub struct Fresh;

impl Fresh {
    #[hotpath::measure]
    fn first(&self) -> u64 {
        black_box(6)
    }
}

// Makes the first `Fresh::first` call, which reads and caches its name.
// Under `hotpath-alloc` nothing is billed to this caller.
#[hotpath::measure]
fn first_call() -> u64 {
    Fresh.first()
}

#[tokio::main(flavor = "current_thread")]
#[hotpath::main]
async fn main() {
    let rev = std::env::args().nth(1).as_deref() == Some("rev");

    let mut widget = Widget::new(1);
    black_box(widget.by_ref());
    widget.by_mut();
    black_box(widget.by_ref_async().await);
    black_box(widget.labeled());
    black_box(widget.explicit());
    black_box(widget.overridden());
    black_box(widget.remote());
    let _ = format!("{widget}");
    let _ = format!("{widget:?}");
    let _ = format!("{widget:?}");

    let run = [
        || Worker { value: 1u8 }.run(),
        || Worker { value: 2u32 }.run(),
    ];
    for i in 0..2 {
        black_box(run[if rev { 1 - i } else { i }]());
    }

    let text = String::from("hello");
    black_box((&Borrowed { text: &text }).render());
    black_box(Gear.turn());
    black_box((&Gear).render());
    black_box(free_fn());
    black_box(first_call());
    black_box(Box::new(widget).boxed());
}

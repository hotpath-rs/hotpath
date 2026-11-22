//! Bare `#[hotpath::measure]` on methods with a receiver reports
//! `module::Type::fn`, inferred from `Self`, for every receiver shape, trait
//! impls on reference types and generic types. Associated functions without
//! a receiver keep `module::fn`, an explicit `impl_type` wins, and gives the
//! same key as the inferred name.
//!
//! Run with:
//!   cargo run -p test-tokio-async --example infer_impl_type --features hotpath
//!   cargo run -p test-tokio-async --example infer_impl_type --features hotpath-alloc

use std::hint::black_box;

struct Widget {
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
    async fn by_ref_async(&self) -> u64 {
        black_box(self.value)
    }

    #[hotpath::measure]
    fn constructor(value: u64) -> Self {
        Self { value }
    }

    #[hotpath::measure(impl_type = "Other")]
    fn overridden(&self) -> u64 {
        black_box(self.value)
    }

    // Same key as the inferred `Twin::twin` below: both calls merge.
    #[hotpath::measure(impl_type = "Widget")]
    fn twin(&self) -> u64 {
        black_box(self.value)
    }
}

trait Twin {
    fn twin(&self) -> u64;
}

impl Twin for Widget {
    #[hotpath::measure]
    fn twin(&self) -> u64 {
        black_box(self.value)
    }
}

struct Borrowed<'a> {
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

struct Plain;

impl Render for &Plain {
    #[hotpath::measure]
    fn render(self) -> usize {
        black_box(0)
    }
}

#[hotpath::measure_all]
impl Render for &Widget {
    fn render(self) -> usize {
        black_box(self.value as usize)
    }
}

struct Painter<F> {
    paint: F,
}

impl<F: Fn() -> u64> Painter<F> {
    #[hotpath::measure]
    fn paint(&self) -> u64 {
        (self.paint)()
    }
}

struct Fresh;

impl Fresh {
    #[hotpath::measure]
    fn first(&self) -> u64 {
        black_box(1)
    }
}

// Makes the first `Fresh::first` call, which builds and caches its name.
// Under `hotpath-alloc` that allocation is not billed to this caller.
#[hotpath::measure]
fn first_call() -> u64 {
    Fresh.first()
}

#[tokio::main(flavor = "current_thread")]
#[hotpath::main]
async fn main() {
    let mut widget = Widget::constructor(1);
    black_box(widget.by_ref());
    widget.by_mut();
    black_box(widget.by_ref_async().await);
    black_box(widget.overridden());
    black_box(Widget::twin(&widget));
    black_box(Twin::twin(&widget));
    black_box((&widget).render());

    let text = String::from("hello");
    black_box((&Borrowed { text: &text }).render());
    black_box((&Plain).render());

    let offset = black_box(2);
    black_box(Painter { paint: || 1 }.paint());
    black_box(
        Painter {
            paint: move || offset,
        }
        .paint(),
    );

    black_box(first_call());

    black_box(Box::new(widget).boxed());
}

//! Names `#[hotpath::measure]` reports for every function shape: one name per
//! source function, whatever instantiation or call order. Run with `rev` to
//! call every multi-type site in the opposite order; the names must not change.
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
    fn by_value(self) -> u64 {
        black_box(self.value)
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

    #[hotpath::measure(future = true)]
    async fn tracked(&self) -> u64 {
        black_box(self.value)
    }

    #[hotpath::measure(label = "widget_labeled")]
    fn labeled(&self) -> u64 {
        black_box(self.value)
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

pub struct Gadget;

trait Defaulted {
    #[hotpath::measure]
    fn hello(&self) -> u64 {
        black_box(1)
    }
}

impl Defaulted for Widget {}
impl Defaulted for Gadget {}

trait Blanket {
    fn blanket(&self) -> u64;
}

impl<T> Blanket for T {
    #[hotpath::measure]
    fn blanket(&self) -> u64 {
        black_box(2)
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

pub struct Multi<T>(T);

impl Multi<u8> {
    #[hotpath::measure]
    fn same(&self) -> u64 {
        black_box(self.0.into())
    }
}

impl Multi<u16> {
    #[hotpath::measure]
    fn same(&self) -> u64 {
        black_box(self.0.into())
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

pub struct Wrapper<T>(T);

impl<T> Render for Wrapper<T> {
    #[hotpath::measure]
    fn render(self) -> usize {
        black_box(1)
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

// Same-named fns nested in two functions.
fn outer_a() -> u64 {
    #[hotpath::measure]
    fn helper() -> u64 {
        black_box(6)
    }
    helper()
}

fn outer_b() -> u64 {
    #[hotpath::measure]
    fn helper() -> u64 {
        black_box(7)
    }
    helper()
}

// Same-named fns in two closures of one function print one path, so the
// second site to run gets its line and column appended.
fn in_closures() -> u64 {
    let first = || {
        #[hotpath::measure]
        fn nested() -> u64 {
            black_box(8)
        }
        nested()
    };
    let second = || {
        #[hotpath::measure]
        fn nested() -> u64 {
            black_box(9)
        }
        nested()
    };
    first() + second()
}

// Two types named `Item` from different modules, impls in one module: the
// second site to run gets its full path.
mod first {
    pub struct Item;
}
mod second {
    pub struct Item;
}

impl first::Item {
    #[hotpath::measure]
    fn get(&self) -> u64 {
        black_box(10)
    }
}

impl second::Item {
    #[hotpath::measure]
    fn get(&self) -> u64 {
        black_box(11)
    }
}

pub struct Fresh;

impl Fresh {
    #[hotpath::measure]
    fn first(&self) -> u64 {
        black_box(12)
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
    let rev = std::env::args().nth(1).as_deref() == Some("rev");

    let mut widget = Widget::new(1);
    black_box(widget.by_ref());
    widget.by_mut();
    black_box(widget.by_ref_async().await);
    black_box(widget.tracked().await);
    black_box(widget.labeled());
    let _ = format!("{widget}");
    let _ = format!("{widget:?}");
    let _ = format!("{widget:?}");

    let hello = [|| Widget::new(1).hello(), || Gadget.hello()];
    let blanket = [|| 1u8.blanket(), || "text".blanket()];
    let run = [
        || Worker { value: 1u8 }.run(),
        || Worker { value: 2u32 }.run(),
    ];
    for i in 0..2 {
        let i = if rev { 1 - i } else { i };
        black_box(hello[i]());
        black_box(blanket[i]());
        black_box(run[i]());
    }
    black_box(Gadget.hello());

    black_box(Multi(1u8).same());
    black_box(Multi(1u16).same());
    let text = String::from("hello");
    black_box((&Borrowed { text: &text }).render());
    black_box(Wrapper(1u8).render());
    black_box(Wrapper("x").render());
    black_box(Gear.turn());
    black_box((&Gear).render());
    black_box(free_fn());
    black_box(outer_a() + outer_b());
    black_box(in_closures());
    black_box(first::Item.get());
    black_box(second::Item.get());
    black_box(first_call());

    black_box(Box::new(widget).boxed());
    black_box(Widget::new(2).by_value());
}

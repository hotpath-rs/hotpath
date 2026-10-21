//! Run with:
//!   cargo run -p test-tokio-async --example measure_all_impl_return_closure --features hotpath

struct Worktree(String);

#[hotpath::measure_all]
impl Worktree {
    fn label<'a>(&'a self) -> impl Fn() -> &'a str + 'a {
        || self.0.as_str()
    }
}

impl Worktree {
    #[hotpath::measure(log = true)]
    fn label_logged<'a>(&'a self) -> &'a str {
        self.0.as_str()
    }

    // The value is coerced to the declared return type inside the `Option`
    // (`&String` to `&str`), which the logging closure must keep doing.
    #[hotpath::measure(log = true)]
    fn label_parts(&self) -> Option<(usize, &str)> {
        Some((self.0.len(), &self.0))
    }
}

fn main() {
    let worktree = Worktree("local-worktree".to_string());
    let _ = worktree.label()();
    let _ = worktree.label_logged();
    let _ = worktree.label_parts();
}

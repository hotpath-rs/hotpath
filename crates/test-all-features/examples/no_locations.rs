//! Run with:
//!   cargo run -p test-all-features --example no_locations --features hotpath-cloud
//!
//! A guard built by hand around code that registers no source location: no
//! measured function, channel, lock or future. The report then has nothing to
//! verify the source checkout with, and must still carry the policy file of
//! the repository it ran in.

fn main() {
    let _guard = hotpath::HotpathGuardBuilder::new("main").build();
    std::hint::black_box(std::thread::current().id());
}

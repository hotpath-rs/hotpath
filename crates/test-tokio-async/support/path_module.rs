//! Loaded by `examples/locations.rs` through `#[path = "../support/..."]`, so
//! `file!()` here keeps a `..` segment the report must resolve.

#[hotpath::measure]
pub(crate) fn path_module_function() {
    std::thread::sleep(std::time::Duration::from_millis(1));
}

//! Run with:
//!   cargo run -p test-tokio-async --example custom_allocator --features hotpath-alloc

#[cfg(feature = "hotpath-alloc")]
mod alloc_demo {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::atomic::{AtomicU64, Ordering};

    static ALLOC_CALLS: AtomicU64 = AtomicU64::new(0);
    static ALLOC_ZEROED_CALLS: AtomicU64 = AtomicU64::new(0);
    static REALLOC_CALLS: AtomicU64 = AtomicU64::new(0);

    pub struct TestAllocator;

    // SAFETY: pure pass-through to `System` - pointers and layouts are
    // forwarded unchanged, so `System`'s GlobalAlloc guarantees carry over.
    unsafe impl GlobalAlloc for TestAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            ALLOC_CALLS.fetch_add(1, Ordering::Relaxed);
            // SAFETY: caller upholds GlobalAlloc's contract for `layout`.
            unsafe { System.alloc(layout) }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            // SAFETY: caller guarantees `ptr` was allocated by this allocator
            // with `layout`, i.e. by `System`.
            unsafe { System.dealloc(ptr, layout) }
        }

        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            ALLOC_ZEROED_CALLS.fetch_add(1, Ordering::Relaxed);
            // SAFETY: caller upholds GlobalAlloc's contract for `layout`.
            unsafe { System.alloc_zeroed(layout) }
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            REALLOC_CALLS.fetch_add(1, Ordering::Relaxed);
            // SAFETY: caller guarantees `ptr` was allocated by this allocator
            // with `layout` and a valid `new_size`, i.e. by `System`.
            unsafe { System.realloc(ptr, layout, new_size) }
        }
    }

    #[hotpath::measure]
    fn alloc_work() {
        let buf = vec![0u8; 1024];
        std::hint::black_box(&buf);
    }

    #[hotpath::measure]
    fn realloc_work() {
        let mut buf: Vec<u8> = Vec::with_capacity(512);
        buf.resize(512, 1);
        buf.reserve(4096);
        std::hint::black_box(&buf);
    }

    #[hotpath::main(allocator = TestAllocator)]
    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let alloc_before = ALLOC_CALLS.load(Ordering::Relaxed);
        let zeroed_before = ALLOC_ZEROED_CALLS.load(Ordering::Relaxed);
        alloc_work();
        assert!(
            ALLOC_CALLS.load(Ordering::Relaxed) > alloc_before,
            "custom allocator should observe an allocation delta"
        );
        assert!(
            ALLOC_ZEROED_CALLS.load(Ordering::Relaxed) > zeroed_before,
            "vec![0; n] should hit the custom allocator's native alloc_zeroed"
        );

        let realloc_before = REALLOC_CALLS.load(Ordering::Relaxed);
        realloc_work();
        assert!(
            REALLOC_CALLS.load(Ordering::Relaxed) > realloc_before,
            "Vec growth should hit the custom allocator's native realloc"
        );

        Ok(())
    }
}

#[cfg(feature = "hotpath-alloc")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    alloc_demo::run()
}

#[cfg(not(feature = "hotpath-alloc"))]
fn main() {}

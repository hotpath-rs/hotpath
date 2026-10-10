use std::sync::atomic::{AtomicU64, Ordering};

/// Per-call totals of a measured async function, fed by the poll wrapper
/// (`futures::wrapper`) around every poll of the body and read by the async
/// guard when the call ends.
#[derive(Debug, Default)]
pub(crate) struct AsyncCallBridge {
    #[cfg(feature = "hotpath-alloc")]
    bytes_total: AtomicU64,
    #[cfg(feature = "hotpath-alloc")]
    count_total: AtomicU64,
    /// Summed inclusive durations of the measured calls that returned during
    /// the body's polls (`HOTPATH_TIME_EXCLUSIVE`).
    children_ns: AtomicU64,
}

impl AsyncCallBridge {
    #[cfg(feature = "hotpath-alloc")]
    #[inline]
    pub(crate) fn add_alloc(&self, bytes: u64, count: u64) {
        self.bytes_total.fetch_add(bytes, Ordering::Relaxed);
        self.count_total.fetch_add(count, Ordering::Relaxed);
    }

    #[cfg(not(feature = "hotpath-alloc"))]
    #[inline]
    pub(crate) fn add_alloc(&self, _bytes: u64, _count: u64) {}

    #[cfg(feature = "hotpath-alloc")]
    #[inline]
    pub(crate) fn alloc_snapshot(&self) -> (Option<u64>, Option<u64>) {
        (
            Some(self.bytes_total.load(Ordering::Relaxed)),
            Some(self.count_total.load(Ordering::Relaxed)),
        )
    }

    /// Polls of one future never overlap, so a load + store pair is enough.
    #[inline]
    pub(crate) fn add_children_ns(&self, ns: u64) {
        let total = self.children_ns.load(Ordering::Relaxed);
        self.children_ns
            .store(total.saturating_add(ns), Ordering::Relaxed);
    }

    #[inline]
    pub(crate) fn children_ns(&self) -> u64 {
        self.children_ns.load(Ordering::Relaxed)
    }
}

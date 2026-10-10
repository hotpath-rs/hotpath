use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Measured async children of a call that started and have not returned yet.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ChildrenInFlight {
    pub(crate) count: u32,
    /// When `count` last left zero, in nanoseconds since the profiling start.
    pub(crate) since_ns: u64,
}

/// Per-call totals of a measured async function, fed by the poll wrapper
/// (`futures::wrapper`) around every poll of the body and read by the async
/// guard when the call ends.
#[derive(Debug, Default)]
pub(crate) struct AsyncCallBridge {
    #[cfg(feature = "hotpath-alloc")]
    bytes_total: AtomicU64,
    #[cfg(feature = "hotpath-alloc")]
    count_total: AtomicU64,
    /// Time covered by the measured calls that returned during the body's
    /// polls (`HOTPATH_TIME_EXCLUSIVE`).
    children_ns: AtomicU64,
    in_flight_count: AtomicU32,
    in_flight_since_ns: AtomicU64,
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

    #[inline]
    pub(crate) fn children_in_flight(&self) -> ChildrenInFlight {
        ChildrenInFlight {
            count: self.in_flight_count.load(Ordering::Relaxed),
            since_ns: self.in_flight_since_ns.load(Ordering::Relaxed),
        }
    }

    /// Stores what one poll collected. Polls of one future never overlap, so
    /// plain loads and stores are enough.
    #[inline]
    pub(crate) fn poll_finished(&self, children_ns: u64, in_flight: ChildrenInFlight) {
        let total = self.children_ns.load(Ordering::Relaxed);
        self.children_ns
            .store(total.saturating_add(children_ns), Ordering::Relaxed);
        self.in_flight_count
            .store(in_flight.count, Ordering::Relaxed);
        self.in_flight_since_ns
            .store(in_flight.since_ns, Ordering::Relaxed);
    }

    /// Time covered by children up to `end_ns`, the end of the call. Children
    /// still in flight then (the call was cancelled) cover it to the end.
    #[inline]
    pub(crate) fn children_covered_ns(&self, end_ns: u64) -> u64 {
        crate::lib_on::functions::exclusive::covered_ns(
            self.children_ns.load(Ordering::Relaxed),
            self.children_in_flight(),
            || end_ns,
        )
    }
}

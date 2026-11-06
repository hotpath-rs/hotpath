#[path = "common/support.rs"]
mod common;

#[cfg(all(test, feature = "hotpath"))]
mod tests {
    use crate::common::example::Example;

    // cargo run -p test-all-features --example wrapper_debug --features hotpath
    #[test]
    fn test_wrappers_implement_debug() {
        let stdout = Example::new("test-all-features", "wrapper_debug").stdout();

        // Each wrapper prints its inner value plus the registration id.
        for expected in [
            "std_tx: Sender { inner: Sender { .. }, id: ",
            "std_rx: Receiver { inner: Receiver { .. }, id: ",
            "std_sync_tx: SyncSender { inner: SyncSender { .. }, id: ",
            "crossbeam_tx: Sender { inner: Sender { .. }, id: ",
            "crossbeam_rx: Receiver { inner: Receiver { .. }, id: ",
            "flume_tx: Sender { inner: Sender, id: ",
            "flume_rx: Receiver { inner: Receiver, id: ",
            "asc_tx: Sender { inner: Sender { .. }, id: ",
            "asc_rx: Receiver { inner: Receiver { .. }, id: ",
            "tokio_tx: Sender { inner: Sender { chan: ",
            "tokio_rx: Receiver { inner: Receiver { chan: ",
            "tokio_unbounded_tx: UnboundedSender { inner: UnboundedSender { chan: ",
            "ftc_tx: Sender { inner: Sender { closed: false }, id: ",
            "ftc_rx: Receiver { inner: Receiver { closed: false }, id: ",
            "std_mutex: Mutex { inner: Mutex { data: 1, poisoned: false, .. }, id: ",
            "std_rw_lock: RwLock { inner: RwLock { data: 2, poisoned: false, .. }, id: ",
            "parking_lot_mutex: Mutex { inner: Mutex { data: 3 }, id: ",
            "parking_lot_rw_lock: RwLock { inner: RwLock { data: 4 }, id: ",
            "async_lock_mutex: Mutex { inner: Mutex { data: 5 }, id: ",
            "async_lock_rw_lock: RwLock { inner: RwLock { value: 6 }, id: ",
            "tokio_mutex: Mutex { inner: Mutex { data: 7 }, id: ",
            "tokio_rw_lock: RwLock { inner: RwLock { data: 8 }, id: ",
            "InstrumentedStream { inner: Iter { iter: IntoIter([1]) }, id: ",
            "InstrumentedFuture { inner: Ready(Some(1)), id: ",
            "InstrumentedIo { inner: Cursor { inner: [], pos: 0 }, id: ",
        ] {
            assert!(
                stdout.contains(expected),
                "missing `{expected}` in:\n{stdout}"
            );
        }
    }
}

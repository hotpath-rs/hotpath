//! Instrumented wrapper for [`async_lock::RwLock`].

use async_lock::RwLock as AlRwLock;

use crate::instant::Instant;
use crate::rw_locks::{
    elapsed_nanos, register_rw_lock, send_rw_lock_event, wait_stamp, InstrumentRwLock, RwLockEvent,
    RwLockKind,
};

/// Instrumented drop-in replacement for [`async_lock::RwLock`].
///
/// Not constructed directly - use the [`rw_lock!`](crate::rw_lock) macro.
pub struct RwLock<T> {
    inner: AlRwLock<T>,
    id: u32,
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure_all)]
impl<T> RwLock<T> {
    /// Drop-in constructor for the `hotpath::wrap` prefix migration. Captures the
    /// caller location as the column-including registration key.
    #[track_caller]
    #[deprecated(note = "construct via the hotpath::rw_lock! macro instead of new()")]
    pub fn new(value: T) -> Self {
        let loc = std::panic::Location::caller();
        let key: &'static str =
            Box::leak(format!("{}:{}:{}", loc.file(), loc.line(), loc.column()).into_boxed_str());
        crate::lib_on::locations::register_caller_location(key, loc);
        Self::__new_instrumented(AlRwLock::new(value), key, None)
    }

    #[doc(hidden)]
    pub fn __new_instrumented(
        inner: AlRwLock<T>,
        source: &'static str,
        label: Option<String>,
    ) -> Self {
        let id = register_rw_lock::<T>(source, label);
        Self { inner, id }
    }

    pub async fn read(&self) -> RwLockReadGuard<'_, T> {
        // Stamp before acquisition to measure wait time; the guard then measures acquire time.
        let wait_start = wait_stamp();
        let inner = self.inner.read().await;
        self.read_guard(inner, wait_start.map(elapsed_nanos))
    }

    pub fn try_read(&self) -> Option<RwLockReadGuard<'_, T>> {
        let wait_start = wait_stamp();
        let inner = self.inner.try_read();
        inner.map(|inner| self.read_guard(inner, wait_start.map(elapsed_nanos)))
    }

    pub async fn write(&self) -> RwLockWriteGuard<'_, T> {
        let wait_start = wait_stamp();
        let inner = self.inner.write().await;
        self.write_guard(inner, wait_start.map(elapsed_nanos))
    }

    pub fn try_write(&self) -> Option<RwLockWriteGuard<'_, T>> {
        let wait_start = wait_stamp();
        let inner = self.inner.try_write();
        inner.map(|inner| self.write_guard(inner, wait_start.map(elapsed_nanos)))
    }

    pub fn into_inner(self) -> T {
        self.inner.into_inner()
    }

    pub fn get_mut(&mut self) -> &mut T {
        self.inner.get_mut()
    }

    fn read_guard<'a>(
        &'a self,
        inner: async_lock::RwLockReadGuard<'a, T>,
        wait_nanos: Option<u64>,
    ) -> RwLockReadGuard<'a, T> {
        RwLockReadGuard {
            inner: Some(inner),
            start: wait_nanos.map(|_| Instant::now()),
            wait_nanos,
            id: self.id,
        }
    }

    fn write_guard<'a>(
        &'a self,
        inner: async_lock::RwLockWriteGuard<'a, T>,
        wait_nanos: Option<u64>,
    ) -> RwLockWriteGuard<'a, T> {
        RwLockWriteGuard {
            inner: Some(inner),
            start: wait_nanos.map(|_| Instant::now()),
            wait_nanos,
            id: self.id,
        }
    }
}

/// Guard returned by [`RwLock::read`]. Emits wait and acquire durations on drop.
#[must_use = "if unused the RwLock will immediately unlock"]
pub struct RwLockReadGuard<'a, T> {
    inner: Option<async_lock::RwLockReadGuard<'a, T>>,
    start: Option<Instant>,
    wait_nanos: Option<u64>,
    id: u32,
}

impl<T> std::ops::Deref for RwLockReadGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.inner.as_ref().expect("guard held until drop")
    }
}

impl<T> Drop for RwLockReadGuard<'_, T> {
    fn drop(&mut self) {
        // Release the real lock before stamping/sending so the held duration
        // excludes the event-send cost and the lock frees as early as possible.
        drop(self.inner.take());
        let acquire_nanos = self
            .start
            .map(|start| Instant::now().duration_since(start).as_nanos() as u64);
        send_rw_lock_event(RwLockEvent::Released {
            id: self.id,
            kind: RwLockKind::Read,
            wait_nanos: self.wait_nanos,
            acquire_nanos,
        });
    }
}

/// Guard returned by [`RwLock::write`]. Emits wait and acquire durations on drop.
#[must_use = "if unused the RwLock will immediately unlock"]
pub struct RwLockWriteGuard<'a, T> {
    inner: Option<async_lock::RwLockWriteGuard<'a, T>>,
    start: Option<Instant>,
    wait_nanos: Option<u64>,
    id: u32,
}

impl<T> std::ops::Deref for RwLockWriteGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.inner.as_ref().expect("guard held until drop")
    }
}

impl<T> std::ops::DerefMut for RwLockWriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.inner.as_mut().expect("guard held until drop")
    }
}

impl<T> Drop for RwLockWriteGuard<'_, T> {
    fn drop(&mut self) {
        // Release the real lock before stamping/sending so the held duration
        // excludes the event-send cost and the lock frees as early as possible.
        drop(self.inner.take());
        let acquire_nanos = self
            .start
            .map(|start| Instant::now().duration_since(start).as_nanos() as u64);
        send_rw_lock_event(RwLockEvent::Released {
            id: self.id,
            kind: RwLockKind::Write,
            wait_nanos: self.wait_nanos,
            acquire_nanos,
        });
    }
}

crate::lib_on::impl_lock_debug!(RwLock, RwLockReadGuard, RwLockWriteGuard);

impl<T> InstrumentRwLock for AlRwLock<T> {
    type Output = RwLock<T>;
    fn instrument(self, source: &'static str, label: Option<String>) -> Self::Output {
        RwLock::__new_instrumented(self, source, label)
    }
}

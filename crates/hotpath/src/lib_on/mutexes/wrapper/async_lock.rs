//! Instrumented wrapper for [`async_lock::Mutex`].

use async_lock::Mutex as AlMutex;

use crate::instant::Instant;
use crate::mutexes::{
    elapsed_nanos, register_mutex, send_mutex_event, wait_stamp, InstrumentMutex, MutexEvent,
};

/// Instrumented drop-in replacement for [`async_lock::Mutex`].
///
/// Not constructed directly - use the [`mutex!`](crate::mutex) macro.
pub struct Mutex<T> {
    inner: AlMutex<T>,
    id: u32,
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure_all)]
impl<T> Mutex<T> {
    /// Drop-in constructor for the `hotpath::wrap` prefix migration. Captures the
    /// caller location as the column-including registration key.
    #[track_caller]
    #[deprecated(note = "construct via the hotpath::mutex! macro instead of new()")]
    pub fn new(value: T) -> Self {
        let loc = std::panic::Location::caller();
        let key: &'static str =
            Box::leak(format!("{}:{}:{}", loc.file(), loc.line(), loc.column()).into_boxed_str());
        crate::lib_on::locations::register_caller_location(key, loc);
        Self::__new_instrumented(AlMutex::new(value), key, None)
    }

    #[doc(hidden)]
    pub fn __new_instrumented(
        inner: AlMutex<T>,
        source: &'static str,
        label: Option<String>,
    ) -> Self {
        let id = register_mutex::<T>(source, label);
        Self { inner, id }
    }

    pub async fn lock(&self) -> MutexGuard<'_, T> {
        // Stamp before acquisition to measure wait time; the guard then measures acquire time.
        let wait_start = wait_stamp();
        let inner = self.inner.lock().await;
        self.guard(inner, wait_start.map(elapsed_nanos))
    }

    pub fn try_lock(&self) -> Option<MutexGuard<'_, T>> {
        let wait_start = wait_stamp();
        let inner = self.inner.try_lock();
        inner.map(|inner| self.guard(inner, wait_start.map(elapsed_nanos)))
    }

    pub fn into_inner(self) -> T {
        self.inner.into_inner()
    }

    pub fn get_mut(&mut self) -> &mut T {
        self.inner.get_mut()
    }

    fn guard<'a>(
        &'a self,
        inner: async_lock::MutexGuard<'a, T>,
        wait_nanos: Option<u64>,
    ) -> MutexGuard<'a, T> {
        MutexGuard {
            inner: Some(inner),
            start: wait_nanos.map(|_| Instant::now()),
            wait_nanos,
            id: self.id,
        }
    }
}

/// Guard returned by [`Mutex::lock`]. Emits wait and acquire durations on drop.
#[must_use = "if unused the Mutex will immediately unlock"]
pub struct MutexGuard<'a, T> {
    inner: Option<async_lock::MutexGuard<'a, T>>,
    start: Option<Instant>,
    wait_nanos: Option<u64>,
    id: u32,
}

impl<T> std::ops::Deref for MutexGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.inner.as_ref().expect("guard held until drop")
    }
}

impl<T> std::ops::DerefMut for MutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.inner.as_mut().expect("guard held until drop")
    }
}

impl<T> Drop for MutexGuard<'_, T> {
    fn drop(&mut self) {
        // Release the real lock before stamping/sending so the held duration
        // excludes the event-send cost and the lock frees as early as possible.
        drop(self.inner.take());
        let acquire_nanos = self
            .start
            .map(|start| Instant::now().duration_since(start).as_nanos() as u64);
        send_mutex_event(MutexEvent::Released {
            id: self.id,
            wait_nanos: self.wait_nanos,
            acquire_nanos,
        });
    }
}

crate::lib_on::impl_lock_debug!(Mutex, MutexGuard);

impl<T> InstrumentMutex for AlMutex<T> {
    type Output = Mutex<T>;
    fn instrument(self, source: &'static str, label: Option<String>) -> Self::Output {
        Mutex::__new_instrumented(self, source, label)
    }
}

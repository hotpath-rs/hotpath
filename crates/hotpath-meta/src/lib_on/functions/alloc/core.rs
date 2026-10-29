use std::cell::Cell;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicU8, Ordering};

#[cfg(feature = "axum-0-8")]
use crate::lib_on::caller_stack::RequestAlloc;
use crate::tid::current_tid;

pub(crate) const MAX_DEPTH: usize = 64;

/// Fixed registry capacity; slots are intentionally never reclaimed so that
/// exited threads stay reportable. After this many unique allocating threads,
/// new threads lose per-thread attribution (function-level alloc stats are
/// unaffected); their bytes still count toward the process-wide overflow
/// totals.
const MAX_THREADS: usize = 256;
const SLOT_UNSET: u32 = u32::MAX;
/// Cached slot index of a hidden internal thread, which never gets a slot.
const SLOT_INTERNAL: u32 = u32::MAX - 1;
/// Cached slot index of a thread that found the registry full. Slots are
/// never reclaimed, so it stays full.
const SLOT_OVERFLOW: u32 = u32::MAX - 2;
const THREAD_NAME_LEN: usize = 64;
/// Base of the synthetic tid range for dead threads whose OS tid was recycled
/// by a new thread. The slot's counters and name stay reportable, and each
/// orphaned slot keeps a distinct, stable identity derived from its index, so
/// report consumers can still use the tid as a row identity.
pub(crate) const TID_ORPHANED_BASE: u64 = u64::MAX - MAX_THREADS as u64;

/// Whether a tid is a synthetic identity of an orphaned slot rather than a
/// live OS tid.
#[cfg(feature = "threads")]
pub(crate) fn is_orphaned_tid(tid: u64) -> bool {
    tid >= TID_ORPHANED_BASE
}

#[repr(align(64))]
pub(crate) struct ThreadAllocStats {
    /// Thread ID (0 unused)
    pub(crate) tid: AtomicU64,
    pub(crate) alloc_bytes: AtomicU64,
    pub(crate) dealloc_bytes: AtomicU64,
    /// Length of `name`; 0 = no name captured. Stored with Release after the
    /// name bytes, so a reader observing a non-zero length sees the full name.
    name_len: AtomicU32,
    name: [AtomicU8; THREAD_NAME_LEN],
}

impl Default for ThreadAllocStats {
    fn default() -> Self {
        Self::new()
    }
}

impl ThreadAllocStats {
    pub(crate) const fn new() -> Self {
        Self {
            tid: AtomicU64::new(0),
            alloc_bytes: AtomicU64::new(0),
            dealloc_bytes: AtomicU64::new(0),
            name_len: AtomicU32::new(0),
            name: [const { AtomicU8::new(0) }; THREAD_NAME_LEN],
        }
    }

    fn store_name(&self, name: &[u8]) {
        if name.is_empty() {
            return;
        }
        for (i, &b) in name.iter().enumerate() {
            self.name[i].store(b, Ordering::Relaxed);
        }
        self.name_len.store(name.len() as u32, Ordering::Release);
    }

    fn name(&self) -> Option<String> {
        let len = self.name_len.load(Ordering::Acquire) as usize;
        if len == 0 || len > THREAD_NAME_LEN {
            return None;
        }
        let bytes: Vec<u8> = self.name[..len]
            .iter()
            .map(|b| b.load(Ordering::Relaxed))
            .collect();
        String::from_utf8(bytes).ok()
    }
}

#[allow(clippy::declare_interior_mutable_const)]
static THREAD_ALLOC_STATS: [ThreadAllocStats; MAX_THREADS] = {
    const INIT: ThreadAllocStats = ThreadAllocStats::new();
    [INIT; MAX_THREADS]
};

static THREAD_TRACKING_ENABLED: AtomicU64 = AtomicU64::new(0);

/// Bytes allocated/deallocated by threads that never got a registry slot
/// (registry full). Keeps process-wide totals accurate past `MAX_THREADS`.
static OVERFLOW_ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);
static OVERFLOW_DEALLOC_BYTES: AtomicU64 = AtomicU64::new(0);

/// Initialize the thread allocation tracking system
#[cfg_attr(not(feature = "threads"), allow(dead_code))]
pub(crate) fn init_thread_alloc_tracking() {
    // Read inside the allocator from here on, where a first-use env lookup
    // would allocate and re-enter it.
    std::sync::LazyLock::force(&crate::lib_on::SHOW_INTERNAL_THREADS);
    THREAD_TRACKING_ENABLED.store(1, Ordering::Release);
}

/// Get allocation stats for a thread
#[cfg_attr(not(feature = "threads"), allow(dead_code))]
pub(crate) fn get_thread_alloc_stats(os_tid: u64) -> Option<(u64, u64)> {
    if THREAD_TRACKING_ENABLED.load(Ordering::Acquire) == 0 {
        return None;
    }

    for slot in &THREAD_ALLOC_STATS {
        let slot_tid = slot.tid.load(Ordering::Acquire);
        if slot_tid == os_tid {
            return Some((
                slot.alloc_bytes.load(Ordering::Relaxed),
                slot.dealloc_bytes.load(Ordering::Relaxed),
            ));
        }
        if slot_tid == 0 {
            break;
        }
    }
    None
}

/// Where a thread's per-thread allocation stats go.
enum ThreadSlot {
    Tracked(&'static ThreadAllocStats),
    /// Registry full: counted only in the process-wide overflow totals.
    Overflow,
    /// A hidden internal thread: not counted at all.
    Internal,
}

#[inline]
fn thread_slot() -> ThreadSlot {
    THREAD_ALLOC_SLOT_IDX.with(|slot_idx| match slot_idx.get() {
        SLOT_UNSET => thread_slot_slow(slot_idx),
        SLOT_INTERNAL => ThreadSlot::Internal,
        SLOT_OVERFLOW => ThreadSlot::Overflow,
        idx => ThreadSlot::Tracked(&THREAD_ALLOC_STATS[idx as usize]),
    })
}

/// One-time per-thread registration, running on the thread itself on its
/// first tracked allocation. Captures the thread's own name so it can be
/// reported even after the thread exits, when resolving it via its pthread is
/// no longer possible.
#[cold]
fn thread_slot_slow(slot_idx: &Cell<u32>) -> ThreadSlot {
    let mut buf = [0u8; THREAD_NAME_LEN];
    let name = current_thread_name(&mut buf);

    // hotpath's own threads suspend tracking before they allocate, but the
    // unnamed threads third-party code spawns from them (tiny_http's accept
    // loop and worker pool) cannot, and on Linux inherit the parent's name.
    if crate::lib_on::is_hidden_internal_thread(name) {
        slot_idx.set(SLOT_INTERNAL);
        return ThreadSlot::Internal;
    }

    match get_or_create_slot_index_slow(current_tid(), name) {
        Some(idx) => {
            slot_idx.set(idx as u32);
            ThreadSlot::Tracked(&THREAD_ALLOC_STATS[idx])
        }
        None => {
            slot_idx.set(SLOT_OVERFLOW);
            ThreadSlot::Overflow
        }
    }
}

#[cold]
fn get_or_create_slot_index_slow(tid: u64, name: &[u8]) -> Option<usize> {
    for (idx, slot) in THREAD_ALLOC_STATS.iter().enumerate() {
        let slot_tid = slot.tid.load(Ordering::Acquire);

        if slot_tid == tid {
            // The slow path only runs on a thread's first tracked allocation
            // (afterwards the slot index is cached thread-locally), so finding
            // our tid already registered means an earlier, now dead thread
            // with a recycled OS tid owns this slot. Orphan it - its counters
            // and name stay reportable - and claim a fresh slot below.
            slot.tid
                .store(TID_ORPHANED_BASE + idx as u64, Ordering::Release);
            continue;
        }

        if slot_tid == 0 {
            match slot
                .tid
                .compare_exchange(0, tid, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => {
                    slot.store_name(name);
                    return Some(idx);
                }
                Err(current) if current == tid => return Some(idx),
                Err(_) => continue,
            }
        }
    }
    None
}

/// The calling thread's name, empty when it has none or the platform cannot
/// tell. Does not allocate, so it is safe inside the allocator.
fn current_thread_name(buf: &mut [u8; THREAD_NAME_LEN]) -> &[u8] {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        // SAFETY: pthread_self is always valid for the calling thread and the
        // buffer is valid for the passed length. pthread_getname_np writes a
        // NUL-terminated string and does not allocate.
        let ret = unsafe {
            libc::pthread_getname_np(
                libc::pthread_self(),
                buf.as_mut_ptr() as *mut libc::c_char,
                THREAD_NAME_LEN,
            )
        };
        if ret != 0 {
            return &[];
        }
        let len = buf.iter().position(|&b| b == 0).unwrap_or(0);
        &buf[..len]
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        &buf[..0]
    }
}

/// Name a registered thread was seen with, if any.
#[cfg_attr(not(feature = "threads"), allow(dead_code))]
pub(crate) fn get_registered_thread_name(os_tid: u64) -> Option<String> {
    for slot in &THREAD_ALLOC_STATS {
        let slot_tid = slot.tid.load(Ordering::Acquire);
        if slot_tid == 0 {
            break;
        }
        if slot_tid == os_tid {
            return slot.name();
        }
    }
    None
}

/// `(alloc_bytes, dealloc_bytes)` from threads that never got a registry slot.
#[cfg_attr(not(feature = "threads"), allow(dead_code))]
pub(crate) fn get_overflow_alloc_stats() -> (u64, u64) {
    (
        OVERFLOW_ALLOC_BYTES.load(Ordering::Relaxed),
        OVERFLOW_DEALLOC_BYTES.load(Ordering::Relaxed),
    )
}

/// Snapshot of every registered per-thread allocation slot:
/// `(tid, alloc_bytes, dealloc_bytes, name)`. Slots outlive their threads, so
/// this includes threads that already exited.
#[cfg_attr(not(feature = "threads"), allow(dead_code))]
pub(crate) fn get_registered_thread_stats() -> Vec<(u64, u64, u64, Option<String>)> {
    let mut stats = Vec::new();
    for slot in &THREAD_ALLOC_STATS {
        let slot_tid = slot.tid.load(Ordering::Acquire);
        if slot_tid == 0 {
            break;
        }
        stats.push((
            slot_tid,
            slot.alloc_bytes.load(Ordering::Relaxed),
            slot.dealloc_bytes.load(Ordering::Relaxed),
            slot.name(),
        ));
    }
    stats
}

pub(crate) struct AllocationInfo {
    pub(crate) bytes_total: Cell<u64>,
    pub(crate) count_total: Cell<u64>,
}

impl std::ops::AddAssign for AllocationInfo {
    fn add_assign(&mut self, other: Self) {
        self.bytes_total
            .set(self.bytes_total.get() + other.bytes_total.get());
        self.count_total
            .set(self.count_total.get() + other.count_total.get());
    }
}

pub(crate) struct AllocationInfoStack {
    pub(crate) depth: Cell<u32>,
    pub(crate) elements: [AllocationInfo; MAX_DEPTH],
    pub(crate) tracking_enabled: Cell<bool>,
    /// Running totals of the server request whose route scope is active on
    /// this thread; fed by `pop_alloc_stack`, never by `track_alloc`.
    #[cfg(feature = "axum-0-8")]
    pub(crate) route: Cell<RequestAlloc>,
    #[cfg(feature = "axum-0-8")]
    pub(crate) route_active: Cell<bool>,
}

thread_local! {
    static THREAD_ALLOC_SLOT_IDX: Cell<u32> = const { Cell::new(SLOT_UNSET) };

    pub(crate) static ALLOCATIONS: AllocationInfoStack = const { AllocationInfoStack {
        depth: Cell::new(0),
        elements: [const { AllocationInfo {
            bytes_total: Cell::new(0),
            count_total: Cell::new(0),
        } }; MAX_DEPTH],
        tracking_enabled: Cell::new(true),
        #[cfg(feature = "axum-0-8")]
        route: Cell::new(RequestAlloc::ZERO),
        #[cfg(feature = "axum-0-8")]
        route_active: Cell::new(false),
    } };
}

/// Opens the route scope's allocation window: installs the request's totals
/// carried over from previous polls, so every allocation tracked on this
/// thread until [`route_alloc_exit`] counts towards the request.
///
/// The window is a plain counter rather than a frame on the allocation stack:
/// the route wants the inclusive total, so it needs no bucket of its own, and
/// staying off the stack keeps it independent of guards whose lifetime
/// straddles a poll boundary (`measure_block!` around an `.await`).
#[cfg(feature = "axum-0-8")]
#[inline]
pub(crate) fn route_alloc_enter(carry: RequestAlloc) {
    ALLOCATIONS.with(|stack| {
        stack.route.set(carry);
        stack.route_active.set(true);
    });
}

/// Closes the window opened by [`route_alloc_enter`] and returns the request's
/// totals so far.
#[cfg(feature = "axum-0-8")]
#[inline]
pub(crate) fn route_alloc_exit() -> RequestAlloc {
    ALLOCATIONS.with(|stack| {
        stack.route_active.set(false);
        stack.route.replace(RequestAlloc::ZERO)
    })
}

/// Counts a tracked allocation towards the active route window, if any.
#[cfg(feature = "axum-0-8")]
#[inline]
pub(crate) fn route_alloc_add(stack: &AllocationInfoStack, bytes: u64, count: u64) {
    if stack.route_active.get() {
        let mut total = stack.route.get();
        total.bytes += bytes;
        total.count += count;
        stack.route.set(total);
    }
}

#[cfg(not(feature = "axum-0-8"))]
#[inline]
pub(crate) fn route_alloc_add(_stack: &AllocationInfoStack, _bytes: u64, _count: u64) {}

#[inline]
pub(crate) fn track_alloc(size: usize) {
    let mut tracking_enabled = true;
    ALLOCATIONS.with(|stack| {
        tracking_enabled = stack.tracking_enabled.get();
        if !tracking_enabled {
            return;
        }
        let depth = stack.depth.get() as usize;
        let info = &stack.elements[depth];
        info.bytes_total.set(info.bytes_total.get() + size as u64);
        info.count_total.set(info.count_total.get() + 1);
        route_alloc_add(stack, size as u64, 1);
    });

    if !tracking_enabled {
        return;
    }

    if THREAD_TRACKING_ENABLED.load(Ordering::Relaxed) != 0 {
        match thread_slot() {
            ThreadSlot::Tracked(slot) => {
                slot.alloc_bytes.fetch_add(size as u64, Ordering::Relaxed);
            }
            ThreadSlot::Overflow => {
                OVERFLOW_ALLOC_BYTES.fetch_add(size as u64, Ordering::Relaxed);
            }
            ThreadSlot::Internal => {}
        }
    }
}

#[inline]
pub(crate) fn track_dealloc(size: usize) {
    let tracking_enabled = ALLOCATIONS.with(|stack| stack.tracking_enabled.get());
    if !tracking_enabled {
        return;
    }

    if THREAD_TRACKING_ENABLED.load(Ordering::Relaxed) != 0 {
        match thread_slot() {
            ThreadSlot::Tracked(slot) => {
                slot.dealloc_bytes.fetch_add(size as u64, Ordering::Relaxed);
            }
            ThreadSlot::Overflow => {
                OVERFLOW_DEALLOC_BYTES.fetch_add(size as u64, Ordering::Relaxed);
            }
            ThreadSlot::Internal => {}
        }
    }
}

#[inline]
pub(crate) fn set_alloc_tracking_enabled(enabled: bool) -> bool {
    ALLOCATIONS.with(|stack| {
        let previous = stack.tracking_enabled.get();
        stack.tracking_enabled.set(enabled);
        previous
    })
}

#[inline]
pub(crate) fn suspend_alloc_tracking() -> bool {
    set_alloc_tracking_enabled(false)
}

#[inline]
pub(crate) fn resume_alloc_tracking(previous_enabled: bool) {
    let _ = set_alloc_tracking_enabled(previous_enabled);
}

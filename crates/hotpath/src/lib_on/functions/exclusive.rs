//! Exclusive (self) time mode for function timing, enabled with
//! `HOTPATH_TIME_EXCLUSIVE`: a function reports its own time without the time
//! spent in nested measured functions.
//!
//! Every guard already knows its inclusive duration, so no extra clock reads
//! are needed: exclusive = inclusive - time covered by direct children. The
//! per-thread state below belongs to the innermost open frame, and each frame
//! keeps its parent's state while it is open, so the frame stack lives in the
//! guards themselves and has no depth limit.
//!
//! Sync children run one after another, so their durations are summed. An
//! async function uses the same definition on wall-clock time (time suspended
//! at an `.await` is its own unless a measured child was running), but async
//! children can run concurrently and overlap. They are therefore covered as a
//! union: the stretch from the first async child in flight to the last one
//! returning counts once, in whatever frame they run (the polls of an async
//! parent, or a sync function driving a runtime). Every poll of a measured
//! async body is a frame, and the state moves through the call's
//! `AsyncCallBridge` between polls because consecutive polls can run on
//! different threads.

use std::cell::Cell;
use std::sync::{Arc, LazyLock};

use crate::instant::Instant;
use crate::lib_on::functions::async_bridge::{AsyncCallBridge, ChildrenInFlight};

pub(crate) static TIME_EXCLUSIVE: LazyLock<bool> =
    LazyLock::new(|| crate::shared::env_flag("HOTPATH_TIME_EXCLUSIVE"));

pub(crate) const DESCRIPTION: &str =
    "Exclusive execution time of functions (excluding nested measured calls).";

/// Identity of a frame, for an async child to find the frame it started in.
/// Only compared.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FrameId {
    /// Outside of any frame.
    Thread,
    /// A sync frame, numbered per thread.
    Sync(u64),
    /// A poll of the async call that owns the bridge at this address.
    Poll(usize),
}

#[derive(Clone, Copy)]
struct FrameState {
    id: FrameId,
    /// Time covered by the children that already returned to this frame.
    children_ns: u64,
    in_flight: ChildrenInFlight,
}

impl FrameState {
    #[inline]
    fn new(id: FrameId, in_flight: ChildrenInFlight) -> Self {
        Self {
            id,
            children_ns: 0,
            in_flight,
        }
    }

    /// A sync child returned. Async children in flight already cover it.
    #[inline]
    fn sync_child_returned(mut self, inclusive_ns: u64) -> Self {
        if self.in_flight.count == 0 {
            self.children_ns = self.children_ns.saturating_add(inclusive_ns);
        }
        self
    }

    /// Time covered by children up to `end_ns`, the end of the frame's call.
    /// Async children still in flight then cover it to the end.
    #[inline]
    fn covered_ns(&self, end_ns: impl FnOnce() -> u64) -> u64 {
        covered_ns(self.children_ns, self.in_flight, end_ns)
    }
}

#[inline]
pub(crate) fn covered_ns(
    children_ns: u64,
    in_flight: ChildrenInFlight,
    end_ns: impl FnOnce() -> u64,
) -> u64 {
    if in_flight.count == 0 {
        return children_ns;
    }
    children_ns.saturating_add(end_ns().saturating_sub(in_flight.since_ns))
}

struct ThreadFrames {
    /// State of the innermost open frame on this thread.
    innermost: Cell<FrameState>,
    next_sync_id: Cell<u64>,
}

thread_local! {
    static FRAMES: ThreadFrames = const {
        ThreadFrames {
            innermost: Cell::new(FrameState {
                id: FrameId::Thread,
                children_ns: 0,
                in_flight: ChildrenInFlight { count: 0, since_ns: 0 },
            }),
            next_sync_id: Cell::new(0),
        }
    };
}

/// An open frame. Holds the enclosing frame's state, which the thread-local
/// stops tracking while this frame is the innermost one.
pub(crate) struct ExclusiveFrame {
    parent: FrameState,
}

impl ExclusiveFrame {
    /// `None` when exclusive mode is off. `try_with` because guards can be
    /// built during thread teardown.
    #[inline]
    pub(crate) fn enter() -> Option<Self> {
        if !*TIME_EXCLUSIVE {
            return None;
        }
        FRAMES
            .try_with(|frames| {
                let id = frames.next_sync_id.get();
                frames.next_sync_id.set(id.wrapping_add(1));
                Self {
                    parent: frames.innermost.replace(FrameState::new(
                        FrameId::Sync(id),
                        ChildrenInFlight::default(),
                    )),
                }
            })
            .ok()
    }

    #[inline]
    fn enter_poll(bridge: &AsyncCallBridge) -> Option<Self> {
        let id = FrameId::Poll(std::ptr::from_ref(bridge) as usize);
        FRAMES
            .try_with(|frames| Self {
                parent: frames
                    .innermost
                    .replace(FrameState::new(id, bridge.children_in_flight())),
            })
            .ok()
    }

    /// Closes the frame on the thread that opened it: hands `inclusive_ns` up
    /// to the enclosing frame and returns this frame's exclusive time.
    #[inline]
    fn exit(&self, inclusive_ns: u64, end: Instant) -> Option<u64> {
        FRAMES
            .try_with(|frames| {
                let own = frames
                    .innermost
                    .replace(self.parent.sync_child_returned(inclusive_ns));
                let covered_ns = own.covered_ns(|| crate::lib_on::elapsed_since_start_ns(end));
                inclusive_ns.saturating_sub(covered_ns)
            })
            .ok()
    }

    /// Closes a poll frame and returns its state. Nothing is handed up: the
    /// async call reports to its parent when its guard is built and dropped.
    #[inline]
    fn exit_poll(&self) -> Option<FrameState> {
        FRAMES
            .try_with(|frames| frames.innermost.replace(self.parent))
            .ok()
    }
}

/// Duration a sync guard reports. In exclusive mode a guard dropped on another
/// thread reports no duration: its frame belongs to the thread that opened
/// it, which only loses the children sum of the enclosing frame.
#[inline]
pub(crate) fn sync_duration_ns(
    frame: Option<ExclusiveFrame>,
    start: Option<Instant>,
    end: Instant,
    cross_thread: bool,
) -> Option<u64> {
    let inclusive_ns = start.map(|start| end.duration_since(start).as_nanos() as u64);
    if !*TIME_EXCLUSIVE {
        return inclusive_ns;
    }
    if cross_thread {
        return None;
    }
    let self_ns = frame?.exit(inclusive_ns.unwrap_or(0), end)?;
    inclusive_ns.map(|_| self_ns)
}

/// The exclusive-time side of an async guard. The guard is built and dropped
/// inside its parent's polls (on whichever threads run them), so it never
/// opens a frame of its own: it tells the innermost frame when it starts and
/// ends, and subtracts what its own polls collected in the bridge.
pub(crate) struct AsyncCall {
    bridge: Option<Arc<AsyncCallBridge>>,
    /// The frame the call started in. `None` until `started`, and for calls
    /// that are not timed.
    parent: Option<FrameId>,
}

impl AsyncCall {
    #[inline]
    pub(crate) fn new(bridge: Option<Arc<AsyncCallBridge>>) -> Self {
        Self {
            bridge,
            parent: None,
        }
    }

    #[inline]
    pub(crate) fn bridge(&self) -> Option<&AsyncCallBridge> {
        self.bridge.as_deref()
    }

    /// Registers the call with the innermost frame. `start` is the guard's
    /// start time, `None` when the call is not timed.
    #[inline]
    pub(crate) fn started(&mut self, start: Option<Instant>) {
        let Some(start) = start else { return };
        if !*TIME_EXCLUSIVE {
            return;
        }
        self.parent = FRAMES
            .try_with(|frames| {
                let mut state = frames.innermost.get();
                if state.in_flight.count == 0 {
                    state.in_flight.since_ns = crate::lib_on::elapsed_since_start_ns(start);
                }
                state.in_flight.count += 1;
                frames.innermost.set(state);
                state.id
            })
            .ok();
    }

    /// Duration the guard reports.
    #[inline]
    pub(crate) fn duration_ns(&self, start: Option<Instant>, end: Instant) -> Option<u64> {
        let inclusive_ns = end.duration_since(start?).as_nanos() as u64;
        if !*TIME_EXCLUSIVE {
            return Some(inclusive_ns);
        }
        let end_ns = crate::lib_on::elapsed_since_start_ns(end);
        self.ended(end_ns);
        let covered_ns = self
            .bridge()
            .map_or(0, |bridge| bridge.children_covered_ns(end_ns));
        Some(inclusive_ns.saturating_sub(covered_ns))
    }

    /// Reports the end of the call to the frame it started in. A guard
    /// dropped anywhere else (its future was moved out of the parent) reports
    /// nothing, rather than corrupting an unrelated frame.
    #[inline]
    fn ended(&self, end_ns: u64) {
        let Some(parent) = self.parent else { return };
        let _ = FRAMES.try_with(|frames| {
            let mut state = frames.innermost.get();
            if state.id != parent || state.in_flight.count == 0 {
                return;
            }
            state.in_flight.count -= 1;
            if state.in_flight.count == 0 {
                let stretch_ns = end_ns.saturating_sub(state.in_flight.since_ns);
                state.children_ns = state.children_ns.saturating_add(stretch_ns);
            }
            frames.innermost.set(state);
        });
    }
}

/// Frame around one poll of a measured async body: collects the time covered
/// by children during the poll. Closes on drop, so a panicking poll still
/// restores the enclosing frame.
pub(crate) struct PollFrame<'a> {
    frame: ExclusiveFrame,
    bridge: &'a AsyncCallBridge,
}

impl<'a> PollFrame<'a> {
    /// `None` for futures that are not measured function bodies (no bridge)
    /// and when exclusive mode is off.
    #[inline]
    pub(crate) fn enter(bridge: Option<&'a AsyncCallBridge>) -> Option<Self> {
        let bridge = bridge?;
        if !*TIME_EXCLUSIVE {
            return None;
        }
        let frame = ExclusiveFrame::enter_poll(bridge)?;
        Some(Self { frame, bridge })
    }
}

impl Drop for PollFrame<'_> {
    #[inline]
    fn drop(&mut self) {
        if let Some(state) = self.frame.exit_poll() {
            self.bridge
                .poll_finished(state.children_ns, state.in_flight);
        }
    }
}

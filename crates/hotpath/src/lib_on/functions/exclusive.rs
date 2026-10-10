//! Exclusive (self) time mode for function timing, enabled with
//! `HOTPATH_TIME_EXCLUSIVE`: a function reports its own time without the time
//! spent in nested measured functions.
//!
//! Every guard already knows its inclusive duration, so no extra clock reads
//! are needed: exclusive = inclusive - sum of the direct children's inclusive
//! durations. The per-thread accumulator below holds that sum for the
//! innermost open frame, and each frame keeps its parent's sum while it is
//! open, so the frame stack lives in the guards themselves and has no depth
//! limit.
//!
//! An async function uses the same definition on wall-clock time, so time
//! suspended at an `.await` counts as its own unless a measured child was
//! running. Its children are collected per poll: every poll of a measured async
//! body is a frame, and the sums go through the call's `AsyncCallBridge`
//! because consecutive polls can run on different threads. Children awaited
//! concurrently overlap, so their sum can exceed the parent's duration, which
//! then reports zero.

use std::cell::Cell;
use std::sync::LazyLock;

use crate::instant::Instant;
use crate::lib_on::functions::async_bridge::AsyncCallBridge;

pub(crate) static TIME_EXCLUSIVE: LazyLock<bool> =
    LazyLock::new(|| crate::shared::env_flag("HOTPATH_TIME_EXCLUSIVE"));

pub(crate) const DESCRIPTION: &str =
    "Exclusive execution time of functions (excluding nested measured calls).";

thread_local! {
    /// Summed inclusive durations of the children that already returned to the
    /// innermost open frame on this thread.
    static CHILDREN_NS: Cell<u64> = const { Cell::new(0) };
}

/// An open frame. Holds the enclosing frame's children sum, which the
/// thread-local stops tracking while this frame is the innermost one.
pub(crate) struct ExclusiveFrame {
    parent_children_ns: u64,
}

impl ExclusiveFrame {
    /// `None` when exclusive mode is off. `try_with` because guards can be
    /// built during thread teardown.
    #[inline]
    pub(crate) fn enter() -> Option<Self> {
        if !*TIME_EXCLUSIVE {
            return None;
        }
        CHILDREN_NS
            .try_with(|children| Self {
                parent_children_ns: children.replace(0),
            })
            .ok()
    }

    /// Closes the frame on the thread that opened it: hands `inclusive_ns` up
    /// to the enclosing frame and returns this frame's exclusive time.
    #[inline]
    fn exit(&self, inclusive_ns: u64) -> Option<u64> {
        CHILDREN_NS
            .try_with(|children| {
                let own_children_ns =
                    children.replace(self.parent_children_ns.saturating_add(inclusive_ns));
                inclusive_ns.saturating_sub(own_children_ns)
            })
            .ok()
    }

    /// Closes a poll frame and returns the children it collected. Nothing is
    /// handed up: the async call reports its whole duration when its guard
    /// drops.
    #[inline]
    fn exit_poll(&self) -> Option<u64> {
        CHILDREN_NS
            .try_with(|children| children.replace(self.parent_children_ns))
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
    let self_ns = frame?.exit(inclusive_ns.unwrap_or(0))?;
    inclusive_ns.map(|_| self_ns)
}

/// Duration an async guard reports. The guard drops inside its parent's poll
/// (on whichever thread runs it), so in exclusive mode it hands its inclusive
/// duration to the innermost frame there and subtracts the children its own
/// polls collected in the bridge.
#[inline]
pub(crate) fn async_duration_ns(
    bridge: Option<&AsyncCallBridge>,
    start: Option<Instant>,
    end: Instant,
) -> Option<u64> {
    let inclusive_ns = end.duration_since(start?).as_nanos() as u64;
    if !*TIME_EXCLUSIVE {
        return Some(inclusive_ns);
    }
    let _ =
        CHILDREN_NS.try_with(|children| children.set(children.get().saturating_add(inclusive_ns)));
    let children_ns = bridge.map_or(0, AsyncCallBridge::children_ns);
    Some(inclusive_ns.saturating_sub(children_ns))
}

/// Frame around one poll of a measured async body: collects the children that
/// returned during the poll. Closes on drop, so a panicking poll still
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
        let frame = ExclusiveFrame::enter()?;
        Some(Self { frame, bridge })
    }
}

impl Drop for PollFrame<'_> {
    #[inline]
    fn drop(&mut self) {
        if let Some(children_ns) = self.frame.exit_poll() {
            self.bridge.add_children_ns(children_ns);
        }
    }
}

//! Process-wide perf counters (`hotpath-pmu`): retired instructions, cycles,
//! cache and branch events and a few kernel software events, reported as
//! deltas since the guard was built. Linux only.

#[cfg(target_os = "linux")]
#[path = "pmu/linux.rs"]
mod imp;

#[cfg(not(target_os = "linux"))]
#[path = "pmu/unsupported.rs"]
mod imp;

pub(crate) use imp::{register_profiler_thread, snapshot, start_session};

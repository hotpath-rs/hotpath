//! Callgrind client requests (`VG_USERREQ_TOOL_BASE('C','T') + n` from
//! `callgrind.h`), issued through the generic Valgrind request in
//! `lib_on/valgrind.rs`.

use crate::lib_on::valgrind::request;

const CALLGRIND_BASE: usize = ((b'C' as usize) << 24) | ((b'T' as usize) << 16);
const ZERO_STATS: usize = CALLGRIND_BASE + 1;
const TOGGLE_COLLECT: usize = CALLGRIND_BASE + 2;
const DUMP_STATS_AT: usize = CALLGRIND_BASE + 3;
const START_INSTRUMENTATION: usize = CALLGRIND_BASE + 4;
const STOP_INSTRUMENTATION: usize = CALLGRIND_BASE + 5;

/// Instruments from here on. A real off->on switch resets every thread's
/// collection state to `--collect-atstart` (Callgrind's `unwind_thread`).
pub(crate) fn start_instrumentation() {
    request(0, START_INSTRUMENTATION, 0);
}

/// Drops costs collected so far (program startup, an earlier session).
pub(crate) fn zero_stats() {
    request(0, ZERO_STATS, 0);
}

/// Flips the calling thread's collection state (per thread in Callgrind).
pub(crate) fn toggle_collect() {
    request(0, TOGGLE_COLLECT, 0);
}

/// Writes a dump whose `desc: Trigger:` line names `tag`, then stops
/// instrumenting so report building and upload run at near native speed.
/// The dump is complete on disk when this returns.
pub(crate) fn dump_and_stop(tag: &std::ffi::CStr) {
    request(0, DUMP_STATS_AT, tag.as_ptr() as usize);
    request(0, STOP_INSTRUMENTATION, 0);
}

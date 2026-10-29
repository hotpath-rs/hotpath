//! Callgrind client requests (`VG_USERREQ_TOOL_BASE('C','T') + n` from
//! `callgrind.h`), issued through the generic Valgrind request in
//! `lib_on/valgrind.rs`.

use crate::lib_on::valgrind::request;

const CALLGRIND_BASE: usize = ((b'C' as usize) << 24) | ((b'T' as usize) << 16);
const ZERO_STATS: usize = CALLGRIND_BASE + 1;
const DUMP_STATS_AT: usize = CALLGRIND_BASE + 3;
const START_INSTRUMENTATION: usize = CALLGRIND_BASE + 4;
const STOP_INSTRUMENTATION: usize = CALLGRIND_BASE + 5;

/// Starts a session: instruments from here on and drops costs collected
/// before it (program startup, an earlier session).
pub(crate) fn start_session() {
    request(0, START_INSTRUMENTATION, 0);
    request(0, ZERO_STATS, 0);
}

/// Writes a dump whose `desc: Trigger:` line names `tag`, then stops
/// instrumenting so report building and upload run at near native speed.
/// The dump is complete on disk when this returns.
pub(crate) fn dump_and_stop(tag: &std::ffi::CStr) {
    request(0, DUMP_STATS_AT, tag.as_ptr() as usize);
    request(0, STOP_INSTRUMENTATION, 0);
}

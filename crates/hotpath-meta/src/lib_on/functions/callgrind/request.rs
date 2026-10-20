//! Valgrind client requests: magic instruction sequences that Valgrind's JIT
//! recognises and natively execute as no-ops returning `default`. Codes come
//! from `valgrind.h` (`VG_USERREQ__RUNNING_ON_VALGRIND`) and `callgrind.h`
//! (`VG_USERREQ_TOOL_BASE('C','T') + n`). Only x86_64 Linux is wired up; every
//! other target reports "not under Valgrind".

use std::sync::LazyLock;

const RUNNING_ON_VALGRIND: usize = 0x1001;
const CALLGRIND_BASE: usize = ((b'C' as usize) << 24) | ((b'T' as usize) << 16);
const ZERO_STATS: usize = CALLGRIND_BASE + 1;
const DUMP_STATS_AT: usize = CALLGRIND_BASE + 3;
const START_INSTRUMENTATION: usize = CALLGRIND_BASE + 4;
const STOP_INSTRUMENTATION: usize = CALLGRIND_BASE + 5;

/// Whether the process runs under Valgrind, queried once.
pub(crate) static ON_VALGRIND: LazyLock<bool> =
    LazyLock::new(|| request(0, RUNNING_ON_VALGRIND, 0) > 0);

#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
#[inline(always)]
fn request(default: usize, code: usize, arg1: usize) -> usize {
    let args: [usize; 6] = [code, arg1, 0, 0, 0, 0];
    let mut result = default;
    // The four rotations add up to 128 bits, so rdi ends where it started and
    // needs no clobber; the `xchg rbx, rbx` after them is the request itself.
    unsafe {
        std::arch::asm!(
            "rol rdi, 3",
            "rol rdi, 13",
            "rol rdi, 61",
            "rol rdi, 51",
            "xchg rbx, rbx",
            inout("rdx") result,
            in("rax") args.as_ptr(),
            options(nostack),
        );
    }
    result
}

#[cfg(not(all(target_arch = "x86_64", target_os = "linux")))]
#[inline(always)]
fn request(default: usize, _code: usize, _arg1: usize) -> usize {
    default
}

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

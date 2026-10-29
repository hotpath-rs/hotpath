//! Valgrind client requests: magic instruction sequences that Valgrind's JIT
//! recognises and natively execute as no-ops returning `default`. The
//! `RUNNING_ON_VALGRIND` code comes from `valgrind.h`. Only x86_64 Linux is
//! wired up; every other target reports "not under Valgrind".

use std::sync::LazyLock;

const RUNNING_ON_VALGRIND: usize = 0x1001;

/// Whether the process runs under Valgrind (any tool), queried once.
pub(crate) static ON_VALGRIND: LazyLock<bool> =
    LazyLock::new(|| request(0, RUNNING_ON_VALGRIND, 0) > 0);

#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
#[inline(always)]
pub(crate) fn request(default: usize, code: usize, arg1: usize) -> usize {
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
pub(crate) fn request(default: usize, _code: usize, _arg1: usize) -> usize {
    default
}

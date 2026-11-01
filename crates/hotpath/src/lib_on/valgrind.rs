//! Valgrind client requests: magic instruction sequences that Valgrind's JIT
//! recognises and natively execute as no-ops returning `default`. The
//! `RUNNING_ON_VALGRIND` code comes from `valgrind.h`. Only x86_64 and aarch64
//! Linux are wired up; every other target reports "not under Valgrind".

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

// `PLAT_arm64_linux` in `valgrind.h`: the rotations of x12 add up to 128 bits,
// so it ends where it started and needs no clobber; the `orr x10, x10, x10`
// after them is the request, which reads the args at x4 and writes x3.
#[cfg(all(target_arch = "aarch64", target_os = "linux"))]
#[inline(always)]
pub(crate) fn request(default: usize, code: usize, arg1: usize) -> usize {
    let args: [usize; 6] = [code, arg1, 0, 0, 0, 0];
    let mut result = default;
    unsafe {
        std::arch::asm!(
            "ror x12, x12, #3",
            "ror x12, x12, #13",
            "ror x12, x12, #51",
            "ror x12, x12, #61",
            "orr x10, x10, x10",
            inout("x3") result,
            in("x4") args.as_ptr(),
            options(nostack),
        );
    }
    result
}

#[cfg(not(any(
    all(target_arch = "x86_64", target_os = "linux"),
    all(target_arch = "aarch64", target_os = "linux")
)))]
#[inline(always)]
pub(crate) fn request(default: usize, _code: usize, _arg1: usize) -> usize {
    default
}

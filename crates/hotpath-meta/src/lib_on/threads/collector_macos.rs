//! macOS Mach kernel FFI for thread metrics collection

use crate::json::ThreadMetrics;
use std::mem;

// Mach kernel types and constants (using C naming conventions)
#[allow(non_camel_case_types)]
mod types {
    pub(crate) type kern_return_t = libc::c_int;
    pub(crate) type mach_port_t = libc::c_uint;
    pub(crate) type thread_act_t = mach_port_t;
    pub(crate) type thread_act_array_t = *mut thread_act_t;
    pub(crate) type mach_msg_type_number_t = libc::c_uint;
    pub(crate) type integer_t = libc::c_int;
}

use types::*;

const KERN_SUCCESS: kern_return_t = 0;
const THREAD_BASIC_INFO: libc::c_int = 3;
const MACH_TASK_BASIC_INFO: libc::c_uint = 20;

// Mach thread run states (from <mach/thread_info.h>)
const TH_STATE_RUNNING: i32 = 1;
const TH_STATE_STOPPED: i32 = 2;
const TH_STATE_WAITING: i32 = 3;
const TH_STATE_UNINTERRUPTIBLE: i32 = 4;
const TH_STATE_HALTED: i32 = 5;

fn run_state_to_status(state: i32) -> (String, String) {
    match state {
        TH_STATE_RUNNING => ("Running".to_string(), "1".to_string()),
        TH_STATE_STOPPED => ("Stopped".to_string(), "2".to_string()),
        TH_STATE_WAITING => ("Sleeping".to_string(), "3".to_string()),
        TH_STATE_UNINTERRUPTIBLE => ("Blocked".to_string(), "4".to_string()),
        TH_STATE_HALTED => ("Halted".to_string(), "5".to_string()),
        _ => ("Unknown".to_string(), state.to_string()),
    }
}

#[allow(non_camel_case_types)]
#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct time_value_t {
    seconds: integer_t,
    microseconds: integer_t,
}

#[allow(non_camel_case_types)]
#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct thread_basic_info {
    user_time: time_value_t,
    system_time: time_value_t,
    cpu_usage: integer_t,
    policy: integer_t,
    run_state: integer_t,
    flags: integer_t,
    suspend_count: integer_t,
    sleep_time: integer_t,
}

/// `<mach/task_info.h>` `mach_task_basic_info`; only `resident_size` is read.
#[repr(C)]
struct mach_task_basic_info {
    virtual_size: u64,
    resident_size: u64,
    resident_size_max: u64,
    user_time: [integer_t; 2],
    system_time: [integer_t; 2],
    policy: integer_t,
    suspend_count: integer_t,
}

extern "C" {
    fn mach_task_self() -> mach_port_t;

    fn task_info(
        target_task: mach_port_t,
        flavor: libc::c_uint,
        task_info_out: *mut integer_t,
        task_info_outCnt: *mut mach_msg_type_number_t,
    ) -> kern_return_t;

    fn task_threads(
        target_task: mach_port_t,
        act_list: *mut thread_act_array_t,
        act_list_cnt: *mut mach_msg_type_number_t,
    ) -> kern_return_t;

    fn thread_info(
        target_act: thread_act_t,
        flavor: libc::c_int,
        thread_info_out: *mut integer_t,
        thread_info_outCnt: *mut mach_msg_type_number_t,
    ) -> kern_return_t;

    fn mach_vm_deallocate(target: mach_port_t, address: u64, size: u64) -> kern_return_t;

    fn pthread_from_mach_thread_np(thread: mach_port_t) -> libc::pthread_t;
}

/// Collect per-thread CPU usage metrics for the current process on macOS
pub(crate) fn collect_thread_metrics() -> Result<Vec<ThreadMetrics>, String> {
    // SAFETY: mach_task_self returns the caller's task port. task_threads is
    // handed valid out-pointers and is checked for KERN_SUCCESS before
    // `thread_list`/`thread_count` are used, so the list read stays in bounds.
    // Each listed port satisfies get_thread_info/get_thread_name's contract.
    // The kernel-allocated list is released with mach_vm_deallocate using the
    // size the kernel reported, exactly once.
    unsafe {
        let task = mach_task_self();
        let mut thread_list: thread_act_array_t = std::ptr::null_mut();
        let mut thread_count: mach_msg_type_number_t = 0;

        let kr = task_threads(task, &mut thread_list, &mut thread_count);
        if kr != KERN_SUCCESS {
            return Err(format!("task_threads failed with code: {}", kr));
        }

        let mut metrics = Vec::new();

        for i in 0..thread_count {
            let thread = *thread_list.offset(i as isize);

            match get_thread_info(thread, i as u64) {
                Ok(metric) => metrics.push(metric),
                Err(_) => {
                    // Thread may have exited between listing and querying - this is normal
                }
            }
        }

        let vm_size = (thread_count as usize * mem::size_of::<mach_port_t>()) as u64;
        mach_vm_deallocate(task, thread_list as u64, vm_size);

        Ok(metrics)
    }
}

// SAFETY: `thread` must be a thread port obtained from `task_threads` for the
// current task. A stale port (thread already exited) is fine - thread_info
// then fails and the error is returned.
unsafe fn get_thread_info(thread: thread_act_t, index: u64) -> Result<ThreadMetrics, String> {
    let mut thread_info_data: thread_basic_info = mem::zeroed();
    let mut thread_info_count = (mem::size_of::<thread_basic_info>() / mem::size_of::<integer_t>())
        as mach_msg_type_number_t;

    let kr = thread_info(
        thread,
        THREAD_BASIC_INFO,
        &mut thread_info_data as *mut _ as *mut integer_t,
        &mut thread_info_count,
    );

    if kr != KERN_SUCCESS {
        return Err(format!("thread_info failed with code: {}", kr));
    }

    let os_tid = thread as u64;
    let name = get_thread_name(thread).unwrap_or_else(|| format!("thread_{}", index));
    let (status, status_code) = run_state_to_status(thread_info_data.run_state);
    let cpu_user = thread_info_data.user_time.seconds as f64
        + (thread_info_data.user_time.microseconds as f64 / 1_000_000.0);
    let cpu_sys = thread_info_data.system_time.seconds as f64
        + (thread_info_data.system_time.microseconds as f64 / 1_000_000.0);

    Ok(ThreadMetrics::new(
        os_tid,
        name,
        status,
        status_code,
        cpu_user,
        cpu_sys,
    ))
}

// SAFETY: `thread` must be a thread port obtained from `task_threads` for the
// current task. A stale port yields a null pthread and returns `None`.
unsafe fn get_thread_name(thread: thread_act_t) -> Option<String> {
    let pthread = pthread_from_mach_thread_np(thread);

    if pthread == 0 {
        return None;
    }

    // Try to get pthread name via libc
    let mut name_buf = [0i8; 256];
    if libc::pthread_getname_np(pthread, name_buf.as_mut_ptr(), name_buf.len()) == 0 {
        let len = name_buf.iter().position(|&c| c == 0).unwrap_or(0);
        if len > 0 {
            let name_bytes: Vec<u8> = name_buf[..len].iter().map(|&c| c as u8).collect();
            if let Ok(name) = String::from_utf8(name_bytes) {
                if !name.is_empty() {
                    return Some(name);
                }
            }
        }
    }

    None
}

/// Check whether a thread, identified by its Mach port, is still alive.
#[cfg_attr(not(feature = "hotpath-alloc-meta"), allow(dead_code))]
pub(crate) fn is_thread_alive(os_tid: u64) -> Option<bool> {
    // SAFETY: thread_info only reads the port; an invalid or dead port makes
    // it return an error, which maps to `false`.
    unsafe {
        let mut info: thread_basic_info = mem::zeroed();
        let mut count = (mem::size_of::<thread_basic_info>() / mem::size_of::<integer_t>())
            as mach_msg_type_number_t;
        let kr = thread_info(
            os_tid as thread_act_t,
            THREAD_BASIC_INFO,
            &mut info as *mut _ as *mut integer_t,
            &mut count,
        );
        Some(kr == KERN_SUCCESS)
    }
}

/// Current RSS (Resident Set Size) of the process in bytes.
pub(crate) fn get_rss_bytes() -> Option<u64> {
    // SAFETY: `info` is zeroed storage of the exact layout `task_info` fills
    // for `MACH_TASK_BASIC_INFO`, with `count` set to its size in
    // `integer_t` units; fields are read only after the call reports success.
    unsafe {
        let mut info: mach_task_basic_info = mem::zeroed();
        let mut count = (mem::size_of::<mach_task_basic_info>() / mem::size_of::<integer_t>())
            as mach_msg_type_number_t;
        let kr = task_info(
            mach_task_self(),
            MACH_TASK_BASIC_INFO,
            &mut info as *mut _ as *mut integer_t,
            &mut count,
        );
        (kr == KERN_SUCCESS).then_some(info.resident_size)
    }
}

/// Max RSS of the process since it started, in bytes, as tracked by the
/// kernel (`ru_maxrss`, which macOS reports in bytes).
pub(crate) fn get_rss_bytes_max() -> Option<u64> {
    // SAFETY: `rusage` is zero-initialized with the layout getrusage expects,
    // and its fields are read only after the call reports success.
    unsafe {
        let mut rusage: libc::rusage = mem::zeroed();
        (libc::getrusage(libc::RUSAGE_SELF, &mut rusage) == 0).then_some(rusage.ru_maxrss as u64)
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use crate::lib_on::threads::collector::*;
    use std::time::Duration;

    #[test]
    fn macos_rss_test() {
        let rss = get_rss_bytes().expect("RSS should be available on macOS");
        let max_rss = get_rss_bytes_max().expect("max RSS should be available on macOS");
        assert!(rss > 0);
        assert!(max_rss >= rss, "max {max_rss} below current {rss}");
    }

    #[test]
    fn macos_thread_metrics_smoke_test() {
        let metrics = collect_thread_metrics().expect("collect_thread_metrics should succeed");
        assert!(!metrics.is_empty());

        for m in &metrics {
            assert_ne!(m.os_tid, 0, "os_tid should not be zero");

            assert!(
                m.cpu_user >= 0.0,
                "cpu_user should be non-negative, got {}",
                m.cpu_user
            );
            assert!(
                m.cpu_sys >= 0.0,
                "cpu_sys should be non-negative, got {}",
                m.cpu_sys
            );
            assert!(
                m.cpu_total >= 0.0,
                "cpu_total should be non-negative, got {}",
                m.cpu_total
            );
        }

        std::thread::sleep(Duration::from_millis(10));

        let metrics2 =
            collect_thread_metrics().expect("second collect_thread_metrics should succeed");

        if !metrics.is_empty() && !metrics2.is_empty() {
            use std::collections::HashMap;

            let mut first_map = HashMap::new();
            for m in &metrics {
                first_map.insert(m.os_tid, m.cpu_total);
            }

            for m in &metrics2 {
                if let Some(first_total) = first_map.get(&m.os_tid) {
                    let delta = m.cpu_total - first_total;
                    assert!(
                        delta > -0.1,
                        "cpu_total went backwards too much for tid {}: {} -> {} (Δ={})",
                        m.os_tid,
                        first_total,
                        m.cpu_total,
                        delta
                    );
                }
            }
        }
    }
}

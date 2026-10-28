//! Linux /proc filesystem thread metrics collection

use crate::json::ThreadMetrics;
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

static CLOCK_TICKS: OnceLock<u64> = OnceLock::new();

/// Convert Linux thread state character to unified status name and native code
fn state_to_status(state: &str) -> (String, String) {
    let code = state.to_string();
    let name = match state {
        "R" => "Running ",
        "S" => "Sleeping",
        "D" => "Blocked", // Disk sleep / uninterruptible
        "Z" => "Zombie",
        "T" => "Stopped",
        "t" => "Stopped", // Tracing stop
        "X" | "x" => "Dead",
        "K" => "Wakekill",
        "W" => "Waking",
        "P" => "Parked",
        "I" => "Idle",
        _ => "Unknown",
    };
    (name.to_string(), code)
}

/// Get clock ticks per second for time conversion
fn clock_ticks_per_sec() -> u64 {
    *CLOCK_TICKS.get_or_init(|| {
        // SAFETY: sysconf takes a plain integer and has no preconditions.
        let v = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        if v <= 0 {
            100 // fallback to 100 Hz
        } else {
            v as u64
        }
    })
}

/// Collect per-thread CPU usage metrics for the current process on Linux
pub(crate) fn collect_thread_metrics() -> Result<Vec<ThreadMetrics>, String> {
    let ticks_per_sec = clock_ticks_per_sec() as f64;
    let task_dir = Path::new("/proc/self/task");

    let entries =
        fs::read_dir(task_dir).map_err(|e| format!("Failed to read /proc/self/task: {}", e))?;

    let mut metrics = Vec::new();

    for entry in entries.flatten() {
        let tid_str = entry.file_name();
        let tid_str = tid_str.to_string_lossy();

        if let Ok(tid) = tid_str.parse::<u64>() {
            match get_thread_info(tid, ticks_per_sec) {
                Ok(metric) => metrics.push(metric),
                Err(_) => {
                    // Thread may have exited between listing and querying - this is normal
                }
            }
        }
    }

    Ok(metrics)
}

fn get_thread_info(tid: u64, ticks_per_sec: f64) -> Result<ThreadMetrics, String> {
    let stat_path = format!("/proc/self/task/{}/stat", tid);
    let comm_path = format!("/proc/self/task/{}/comm", tid);

    // Read thread name from comm (max 15 chars, no newline)
    let name = fs::read_to_string(&comm_path)
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| format!("thread_{}", tid));

    let stat_content = fs::read_to_string(&stat_path)
        .map_err(|e| format!("Failed to read {}: {}", stat_path, e))?;

    // Format: "pid (comm) state ... utime(14) stime(15) ...". comm may contain
    // spaces/parens, so split at the last ')'.
    let stat_after_comm = stat_content
        .rfind(')')
        .map(|i| &stat_content[i + 2..]) // Skip ") "
        .ok_or_else(|| "Invalid stat format".to_string())?;

    let fields: Vec<&str> = stat_after_comm.split_whitespace().collect();

    // Fields after comm: [0]=state, [11]=utime, [12]=stime.
    if fields.len() < 13 {
        return Err(format!("stat file has too few fields: {}", fields.len()));
    }

    let (status, status_code) = state_to_status(fields[0]);

    let utime_ticks: u64 = fields[11]
        .parse()
        .map_err(|_| "Failed to parse utime".to_string())?;
    let stime_ticks: u64 = fields[12]
        .parse()
        .map_err(|_| "Failed to parse stime".to_string())?;

    let cpu_user = utime_ticks as f64 / ticks_per_sec;
    let cpu_sys = stime_ticks as f64 / ticks_per_sec;

    Ok(ThreadMetrics::new(
        tid,
        name,
        status,
        status_code,
        cpu_user,
        cpu_sys,
    ))
}

/// Check whether a thread, identified by its kernel TID, is still alive.
#[cfg_attr(not(feature = "hotpath-alloc"), allow(dead_code))]
pub(crate) fn is_thread_alive(os_tid: u64) -> Option<bool> {
    Some(std::path::Path::new(&format!("/proc/self/task/{os_tid}")).exists())
}

/// Current RSS (Resident Set Size) of the process in bytes.
pub(crate) fn get_rss_bytes() -> Option<u64> {
    // Read from /proc/self/statm - second field is RSS in pages
    let statm = fs::read_to_string("/proc/self/statm").ok()?;
    let fields: Vec<&str> = statm.split_whitespace().collect();
    if fields.len() >= 2 {
        let rss_pages: u64 = fields[1].parse().ok()?;
        // SAFETY: sysconf takes a plain integer and has no preconditions.
        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as u64;
        Some(rss_pages * page_size)
    } else {
        None
    }
}

/// Peak RSS of the current program image in bytes, from `VmHWM` in
/// `/proc/self/status`. Unlike `ru_maxrss`, it resets on `execve`, so the
/// peak of a launcher that exec'd into us (e.g. `cargo run` after a build)
/// is not included. Falls back to `ru_maxrss` when procfs is unavailable.
pub(crate) fn get_rss_bytes_max() -> Option<u64> {
    let vm_hwm_kb = fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            let line = status.lines().find(|l| l.starts_with("VmHWM:"))?;
            line.split_whitespace().nth(1)?.parse::<u64>().ok()
        });
    if let Some(kb) = vm_hwm_kb {
        return Some(kb * 1024);
    }

    // ru_maxrss is in kilobytes on Linux.
    // SAFETY: `rusage` is zero-initialized with the layout getrusage expects,
    // and its fields are read only after the call reports success.
    unsafe {
        let mut rusage: libc::rusage = std::mem::zeroed();
        (libc::getrusage(libc::RUSAGE_SELF, &mut rusage) == 0)
            .then_some(rusage.ru_maxrss as u64 * 1024)
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use crate::lib_on::threads::collector::*;
    use std::collections::HashMap;
    use std::time::Duration;

    #[test]
    fn linux_rss_test() {
        let rss = get_rss_bytes().expect("RSS should be available on Linux");
        let max_rss = get_rss_bytes_max().expect("max RSS should be available on Linux");
        assert!(rss > 0);
        assert!(max_rss >= rss, "max {max_rss} below current {rss}");
    }

    #[test]
    fn linux_thread_metrics_smoke_test() {
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

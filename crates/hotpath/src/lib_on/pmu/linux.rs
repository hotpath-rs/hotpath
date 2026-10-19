//! perf_event_open(2) counters, read as three views:
//!
//! - process: one inherited counter set per thread alive when the first
//!   session starts. Every later thread descends from one of them, so the sum
//!   covers the whole process.
//! - profiler: every `hp-*` thread opens its own inherited set when it starts
//!   (`spawn_profiler_thread`), so threads it spawns (tiny_http, tokio
//!   blocking pools) count as profiler work too. The application view is
//!   process minus profiler.
//! - main thread: the guard's thread alone, without inheritance.
//!
//! Counters stay open for the process lifetime; each session reports deltas
//! from a baseline read when its guard was built.
//!
//! On hybrid CPUs (Intel P/E cores) a generic hardware event is bound to one
//! core type, so it only counts while the thread runs on that type.

use std::fs;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::{Mutex, OnceLock, PoisonError};

use crate::json::{JsonPmuCounters, JsonPmuEvent, JsonPmuValue, PmuEvent};

const PERF_TYPE_HARDWARE: u32 = 0;
const PERF_TYPE_SOFTWARE: u32 = 1;
const PERF_FORMAT_TOTAL_TIME_ENABLED: u64 = 1 << 0;
const PERF_FORMAT_TOTAL_TIME_RUNNING: u64 = 1 << 1;
const PERF_FLAG_FD_CLOEXEC: libc::c_ulong = 1 << 3;
const ATTR_FLAG_INHERIT: u64 = 1 << 1;
const ATTR_FLAG_EXCLUDE_KERNEL: u64 = 1 << 5;
const ATTR_FLAG_EXCLUDE_HV: u64 = 1 << 6;

/// `hp-cpu-snapshot` spawns a thread per snapshot, so registrations are
/// capped. Work of threads past the cap is reported as application work.
const MAX_PROFILER_THREADS: usize = 64;

const EVENT_COUNT: usize = PmuEvent::ALL.len();

/// `struct perf_event_attr` up to `config1` (`PERF_ATTR_SIZE_VER0`), which
/// is all counting needs. The kernel accepts any published size.
#[repr(C)]
#[derive(Default)]
struct PerfEventAttr {
    type_: u32,
    size: u32,
    config: u64,
    sample_period: u64,
    sample_type: u64,
    read_format: u64,
    flags: u64,
    wakeup_events: u32,
    bp_type: u32,
    config1: u64,
}

const _: () = assert!(std::mem::size_of::<PerfEventAttr>() == 64);

fn type_and_config(event: PmuEvent) -> (u32, u64) {
    match event {
        PmuEvent::Cycles => (PERF_TYPE_HARDWARE, 0),
        PmuEvent::Instructions => (PERF_TYPE_HARDWARE, 1),
        PmuEvent::CacheReferences => (PERF_TYPE_HARDWARE, 2),
        PmuEvent::CacheMisses => (PERF_TYPE_HARDWARE, 3),
        PmuEvent::BranchInstructions => (PERF_TYPE_HARDWARE, 4),
        PmuEvent::BranchMisses => (PERF_TYPE_HARDWARE, 5),
        PmuEvent::StalledCyclesFrontend => (PERF_TYPE_HARDWARE, 7),
        PmuEvent::StalledCyclesBackend => (PERF_TYPE_HARDWARE, 8),
        PmuEvent::RefCycles => (PERF_TYPE_HARDWARE, 9),
        PmuEvent::TaskClock => (PERF_TYPE_SOFTWARE, 1),
        PmuEvent::PageFaults => (PERF_TYPE_SOFTWARE, 2),
        PmuEvent::ContextSwitches => (PERF_TYPE_SOFTWARE, 3),
        PmuEvent::CpuMigrations => (PERF_TYPE_SOFTWARE, 4),
    }
}

struct Counter {
    fd: OwnedFd,
    user_space_only: bool,
}

/// Raw counter value plus the times the kernel needs to scale it when the
/// event was multiplexed.
#[derive(Clone, Copy, Default)]
struct Reading {
    value: u64,
    enabled_ns: u64,
    running_ns: u64,
    user_space_only: bool,
}

impl Reading {
    fn add(self, other: Reading) -> Reading {
        Reading {
            value: self.value + other.value,
            enabled_ns: self.enabled_ns + other.enabled_ns,
            running_ns: self.running_ns + other.running_ns,
            user_space_only: self.user_space_only || other.user_space_only,
        }
    }

    fn since(self, baseline: Reading) -> Reading {
        Reading {
            value: self.value.saturating_sub(baseline.value),
            enabled_ns: self.enabled_ns.saturating_sub(baseline.enabled_ns),
            running_ns: self.running_ns.saturating_sub(baseline.running_ns),
            user_space_only: self.user_space_only,
        }
    }

    /// The value extrapolated to the full enabled time, or `None` when the
    /// event was enabled but never got a hardware counter.
    fn scaled(self) -> Option<u64> {
        if self.running_ns == 0 {
            return (self.enabled_ns == 0).then_some(self.value);
        }
        if self.running_ns >= self.enabled_ns {
            return Some(self.value);
        }
        let scaled = self.value as u128 * self.enabled_ns as u128 / self.running_ns as u128;
        Some(scaled.min(u64::MAX as u128) as u64)
    }

    fn running_percent(self) -> f64 {
        if self.enabled_ns == 0 {
            100.0
        } else {
            self.running_ns as f64 / self.enabled_ns as f64 * 100.0
        }
    }
}

type Readings = [Result<Reading, String>; EVENT_COUNT];

fn zero_readings() -> Readings {
    std::array::from_fn(|_| Ok(Reading::default()))
}

/// One thread's counters, one slot per `PmuEvent::ALL` entry.
struct CounterSet {
    counters: [Result<Counter, String>; EVENT_COUNT],
}

impl CounterSet {
    /// `tid` 0 is the calling thread.
    fn open(tid: libc::pid_t, inherit: bool) -> Self {
        Self {
            counters: PmuEvent::ALL.map(|event| open_counter(event, tid, inherit)),
        }
    }

    fn any_open(&self) -> bool {
        self.counters.iter().any(Result::is_ok)
    }

    fn read(&self) -> Readings {
        std::array::from_fn(|i| match &self.counters[i] {
            Ok(counter) => read_counter(counter),
            Err(reason) => Err(reason.clone()),
        })
    }
}

/// Sums each event over `sets`. An event that failed on any set is
/// unavailable as a whole rather than silently undercounted.
fn sum_readings<'a>(sets: impl IntoIterator<Item = &'a CounterSet>) -> Readings {
    let mut total = zero_readings();
    for set in sets {
        for (slot, reading) in total.iter_mut().zip(set.read()) {
            *slot = match (std::mem::replace(slot, Ok(Reading::default())), reading) {
                (Ok(acc), Ok(reading)) => Ok(acc.add(reading)),
                (Err(reason), _) | (_, Err(reason)) => Err(reason),
            };
        }
    }
    total
}

/// Hardware events count user space only: kernel work makes instruction
/// counts less repeatable, and `perf_event_paranoid` 2 (the upstream
/// default) allows nothing more. Software events try kernel mode first,
/// because context switches and migrations happen in the kernel and would
/// always read 0 in user space only.
fn open_counter(event: PmuEvent, tid: libc::pid_t, inherit: bool) -> Result<Counter, String> {
    let (type_, config) = type_and_config(event);
    if type_ == PERF_TYPE_SOFTWARE {
        if let Ok(fd) = perf_event_open(type_, config, tid, inherit, false) {
            return Ok(Counter {
                fd,
                user_space_only: false,
            });
        }
    }
    perf_event_open(type_, config, tid, inherit, true)
        .map(|fd| Counter {
            fd,
            user_space_only: true,
        })
        .map_err(|e| describe_open_error(&e))
}

fn perf_event_open(
    type_: u32,
    config: u64,
    tid: libc::pid_t,
    inherit: bool,
    exclude_kernel: bool,
) -> io::Result<OwnedFd> {
    let mut flags = ATTR_FLAG_EXCLUDE_HV;
    if inherit {
        flags |= ATTR_FLAG_INHERIT;
    }
    if exclude_kernel {
        flags |= ATTR_FLAG_EXCLUDE_KERNEL;
    }
    let attr = PerfEventAttr {
        type_,
        size: std::mem::size_of::<PerfEventAttr>() as u32,
        config,
        read_format: PERF_FORMAT_TOTAL_TIME_ENABLED | PERF_FORMAT_TOTAL_TIME_RUNNING,
        flags,
        ..Default::default()
    };
    // SAFETY: `attr` is a fully initialized perf_event_attr whose `size`
    // matches its layout; the kernel only reads it during the call.
    let fd = unsafe {
        libc::syscall(
            libc::SYS_perf_event_open,
            &attr as *const PerfEventAttr,
            tid,
            -1 as libc::c_int,
            -1 as libc::c_int,
            PERF_FLAG_FD_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the kernel just returned this descriptor and nothing else owns it.
    Ok(unsafe { OwnedFd::from_raw_fd(fd as libc::c_int) })
}

fn read_counter(counter: &Counter) -> Result<Reading, String> {
    let mut buf = [0u64; 3];
    // SAFETY: `buf` is valid for writes of its full size and the descriptor
    // stays open for the duration of the call.
    let n = unsafe {
        libc::read(
            counter.fd.as_raw_fd(),
            buf.as_mut_ptr().cast(),
            std::mem::size_of_val(&buf),
        )
    };
    if n != std::mem::size_of_val(&buf) as isize {
        return Err(format!("read failed: {}", io::Error::last_os_error()));
    }
    Ok(Reading {
        value: buf[0],
        enabled_ns: buf[1],
        running_ns: buf[2],
        user_space_only: counter.user_space_only,
    })
}

fn describe_open_error(err: &io::Error) -> String {
    match err.raw_os_error() {
        Some(code @ (libc::EACCES | libc::EPERM)) => {
            let paranoid = fs::read_to_string("/proc/sys/kernel/perf_event_paranoid")
                .ok()
                .and_then(|s| s.trim().parse::<i32>().ok());
            match paranoid {
                Some(level) if level > 2 => format!(
                    "permission denied: kernel.perf_event_paranoid is {level}, \
                     set it to 2 or lower (os error {code})"
                ),
                _ => format!(
                    "permission denied, likely a seccomp filter; in Docker run with \
                     --cap-add PERFMON (os error {code})"
                ),
            }
        }
        Some(code @ (libc::ENOENT | libc::EOPNOTSUPP | libc::ENODEV)) => {
            format!("not supported by this CPU or hypervisor (os error {code})")
        }
        _ => err.to_string(),
    }
}

/// Inherited sets on every thread alive at the first session start.
static PROCESS: OnceLock<Vec<CounterSet>> = OnceLock::new();

/// Inherited sets of `hp-*` threads, each opened by the thread itself.
static PROFILER: Mutex<Vec<CounterSet>> = Mutex::new(Vec::new());

/// Replaced by every guard build, so a session reports only its own window.
static SESSION: Mutex<Option<Session>> = Mutex::new(None);

struct Session {
    main_thread: CounterSet,
    main_thread_baseline: Readings,
    process_baseline: Readings,
    profiler_baseline: Readings,
}

fn open_process_counters() -> Vec<CounterSet> {
    // SAFETY: gettid has no preconditions.
    let own_tid = unsafe { libc::gettid() };
    let mut tids: Vec<libc::pid_t> = fs::read_dir("/proc/self/task")
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|e| e.file_name().to_str()?.parse().ok())
                .collect()
        })
        .unwrap_or_default();
    if !tids.contains(&own_tid) {
        tids.push(own_tid);
    }
    tids.into_iter()
        .map(|tid| (tid, CounterSet::open(tid, true)))
        // A thread that exited after the listing has nothing to count, but
        // the guard's own set is kept so its errors reach the report.
        .filter(|(tid, set)| *tid == own_tid || set.any_open())
        .map(|(_, set)| set)
        .collect()
}

pub(crate) fn register_profiler_thread() {
    let _suspend = crate::lib_on::SuspendAllocTracking::new();
    let mut profiler = PROFILER.lock().unwrap_or_else(PoisonError::into_inner);
    if profiler.len() >= MAX_PROFILER_THREADS {
        return;
    }
    let set = CounterSet::open(0, true);
    if set.any_open() {
        profiler.push(set);
    }
}

pub(crate) fn start_session() {
    let process = PROCESS.get_or_init(open_process_counters);
    let main_thread = CounterSet::open(0, false);
    let main_thread_baseline = main_thread.read();
    let process_baseline = sum_readings(process);
    let profiler_baseline = sum_readings(
        PROFILER
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter(),
    );
    *SESSION.lock().unwrap_or_else(PoisonError::into_inner) = Some(Session {
        main_thread,
        main_thread_baseline,
        process_baseline,
        profiler_baseline,
    });
}

pub(crate) fn snapshot() -> JsonPmuCounters {
    let Some(process) = PROCESS.get() else {
        return JsonPmuCounters::Unavailable {
            reason: "no profiling session has started".to_string(),
        };
    };
    let process_now = sum_readings(process);
    let profiler_now = sum_readings(
        PROFILER
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter(),
    );

    let session = SESSION.lock().unwrap_or_else(PoisonError::into_inner);
    let Some(session) = session.as_ref() else {
        return JsonPmuCounters::Unavailable {
            reason: "no profiling session has started".to_string(),
        };
    };
    let main_thread_now = session.main_thread.read();

    let events = (0..EVENT_COUNT)
        .map(|i| JsonPmuEvent {
            event: PmuEvent::ALL[i],
            value: event_value(
                delta(&process_now[i], &session.process_baseline[i]),
                delta(&profiler_now[i], &session.profiler_baseline[i]),
                delta(&main_thread_now[i], &session.main_thread_baseline[i]),
            ),
        })
        .collect::<Vec<_>>();

    // A systemic failure (permissions, no perf support) hits every event
    // the same way, so it is reported once.
    let mut reasons = events.iter().map(|e| match &e.value {
        JsonPmuValue::Unavailable { reason } => Some(reason),
        JsonPmuValue::Counted { .. } => None,
    });
    if let Some(Some(first)) = reasons.next() {
        if reasons.all(|reason| reason == Some(first)) {
            return JsonPmuCounters::Unavailable {
                reason: first.clone(),
            };
        }
    }
    JsonPmuCounters::Available { events }
}

fn delta(
    now: &Result<Reading, String>,
    baseline: &Result<Reading, String>,
) -> Result<Reading, String> {
    match (now, baseline) {
        (Ok(now), Ok(baseline)) => Ok(now.since(*baseline)),
        (Err(reason), _) | (_, Err(reason)) => Err(reason.clone()),
    }
}

fn event_value(
    process: Result<Reading, String>,
    profiler: Result<Reading, String>,
    main_thread: Result<Reading, String>,
) -> JsonPmuValue {
    let counted = || -> Result<JsonPmuValue, String> {
        const NEVER_SCHEDULED: &str = "never got a hardware counter, all counters were busy";
        let process = process?;
        let profiler = profiler.map_err(|reason| format!("profiler threads: {reason}"))?;
        let process_value = process.scaled().ok_or(NEVER_SCHEDULED)?;
        let profiler_value = profiler.scaled().ok_or(NEVER_SCHEDULED)?;
        Ok(JsonPmuValue::Counted {
            process: process_value,
            // Both sides are scaled estimates when multiplexed, so the
            // difference can dip below zero.
            application: process_value.saturating_sub(profiler_value),
            profiler: profiler_value,
            main_thread: main_thread.ok().and_then(Reading::scaled),
            running_percent: process.running_percent(),
            user_space_only: process.user_space_only,
        })
    };
    counted().unwrap_or_else(|reason| JsonPmuValue::Unavailable { reason })
}

#[cfg(test)]
mod tests {
    use crate::json::PmuEvent;
    use crate::lib_on::pmu::imp::{read_counter, CounterSet, Reading};

    /// `task-clock` is a software event, available even in VMs without a
    /// PMU, so it exercises the open/read/delta path wherever perf events
    /// are allowed at all.
    #[test]
    fn task_clock_grows_with_cpu_work() {
        let set = CounterSet::open(0, false);
        let index = PmuEvent::ALL
            .iter()
            .position(|e| *e == PmuEvent::TaskClock)
            .unwrap();
        let counter = match &set.counters[index] {
            Ok(counter) => counter,
            Err(reason) => {
                eprintln!("skipping, perf events unavailable: {reason}");
                return;
            }
        };
        let read = || -> Reading { read_counter(counter).unwrap() };

        let before = read();
        let mut x = 0u64;
        for i in 0..50_000_000u64 {
            x = std::hint::black_box(x.wrapping_mul(31).wrapping_add(i));
        }
        let cpu_ns = read().since(before).scaled().unwrap();

        assert!(
            cpu_ns > 1_000_000,
            "expected >1ms of CPU time, got {cpu_ns}ns"
        );
    }
}

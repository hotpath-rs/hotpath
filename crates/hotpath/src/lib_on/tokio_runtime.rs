use std::sync::OnceLock;
use std::time::Duration;

use tokio::runtime::Handle;

use crate::json::{JsonRuntimeSnapshot, JsonRuntimeWorker};

static RUNTIME_STATE: OnceLock<()> = OnceLock::new();

const DEFAULT_RUNTIME_INTERVAL_MS: u64 = 1000;

pub fn init_runtime_monitoring(handle: &Handle) {
    let handle = handle.clone();
    RUNTIME_STATE.get_or_init(|| {
        let interval_ms = std::env::var("HOTPATH_TOKIO_RUNTIME_INTERVAL_MS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_RUNTIME_INTERVAL_MS);

        let interval = Duration::from_millis(interval_ms);

        crate::lib_on::spawn_profiler_thread("hp-runtime", move || {
            runtime_loop(handle, interval);
        })
        .expect("Failed to spawn hp-runtime thread");
    });
}

fn runtime_loop(handle: Handle, interval: Duration) {
    loop {
        let metrics = handle.metrics();
        let snapshot = snapshot_from_metrics(&metrics);

        if let Ok(mut lock) = get_snapshot_lock().write() {
            *lock = Some(snapshot);
        }

        std::thread::sleep(interval);
    }
}

fn snapshot_from_metrics(m: &tokio::runtime::RuntimeMetrics) -> JsonRuntimeSnapshot {
    let workers = (0..m.num_workers())
        .map(|i| {
            #[allow(unused_mut)]
            let mut w = JsonRuntimeWorker {
                index: i,
                park_count: m.worker_park_count(i),
                busy_duration_ms: m.worker_total_busy_duration(i).as_millis() as u64,
                poll_count: None,
                steal_count: None,
                steal_operations: None,
                overflow_count: None,
                local_queue_depth: None,
                mean_poll_time_us: None,
            };

            #[cfg(tokio_unstable)]
            {
                w.poll_count = Some(m.worker_poll_count(i));
                w.steal_count = Some(m.worker_steal_count(i));
                w.steal_operations = Some(m.worker_steal_operations(i));
                w.overflow_count = Some(m.worker_overflow_count(i));
                w.local_queue_depth = Some(m.worker_local_queue_depth(i));
                w.mean_poll_time_us = Some(m.worker_mean_poll_time(i).as_micros() as u64);
            }

            w
        })
        .collect();

    #[allow(unused_mut)]
    let mut snapshot = JsonRuntimeSnapshot {
        num_workers: m.num_workers(),
        num_alive_tasks: m.num_alive_tasks(),
        global_queue_depth: m.global_queue_depth(),
        workers,
        num_blocking_threads: None,
        num_idle_blocking_threads: None,
        blocking_queue_depth: None,
        spawned_tasks_count: None,
        remote_schedule_count: None,
        io_driver_fd_registered_count: None,
        io_driver_fd_deregistered_count: None,
        io_driver_ready_count: None,
    };

    #[cfg(tokio_unstable)]
    {
        snapshot.num_blocking_threads = Some(m.num_blocking_threads());
        snapshot.num_idle_blocking_threads = Some(m.num_idle_blocking_threads());
        snapshot.blocking_queue_depth = Some(m.blocking_queue_depth());
        snapshot.spawned_tasks_count = Some(m.spawned_tasks_count());
        snapshot.remote_schedule_count = Some(m.remote_schedule_count());
        snapshot.io_driver_fd_registered_count = Some(m.io_driver_fd_registered_count());
        snapshot.io_driver_fd_deregistered_count = Some(m.io_driver_fd_deregistered_count());
        snapshot.io_driver_ready_count = Some(m.io_driver_ready_count());
    }

    snapshot
}

static LATEST_SNAPSHOT: OnceLock<crate::lib_on::MetaRwLock<Option<JsonRuntimeSnapshot>>> =
    OnceLock::new();

fn get_snapshot_lock() -> &'static crate::lib_on::MetaRwLock<Option<JsonRuntimeSnapshot>> {
    LATEST_SNAPSHOT.get_or_init(|| crate::lib_on::meta_rw_lock!("tokio_runtime_snapshot", None))
}

pub(crate) fn get_runtime_json() -> Option<JsonRuntimeSnapshot> {
    get_snapshot_lock().read().ok()?.clone()
}

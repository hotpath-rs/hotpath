use crate::json::JsonPmuCounters;

pub(crate) fn register_profiler_thread() {}

pub(crate) fn start_session() {}

pub(crate) fn snapshot() -> JsonPmuCounters {
    JsonPmuCounters::Unavailable {
        reason: format!(
            "perf counters are Linux-only (running on {})",
            std::env::consts::OS
        ),
    }
}

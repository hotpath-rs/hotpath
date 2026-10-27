use crossbeam_channel::Receiver;
use hdrhistogram::Histogram;
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::Duration;

use crate::batch::{EventProducer, EventQueueRegistry};
use crate::instant::Instant;

static MEASUREMENT_QUEUES: EventQueueRegistry<Measurement> = EventQueueRegistry::new();

thread_local! {
    static MEASUREMENT_PRODUCER: EventProducer<Measurement> = MEASUREMENT_QUEUES.register();
}

/// Set when an event could not be enqueued (its thread's producer was already
/// torn down); allocation queries fail for the rest of the session.
#[cfg(feature = "hotpath-ordered-drain")]
static LOST_MEASUREMENTS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn add_measurement(m: Measurement) {
    if !MEASUREMENT_QUEUES.is_active() {
        return;
    }
    let _result = MEASUREMENT_PRODUCER.try_with(|producer| producer.push(m));
    #[cfg(feature = "hotpath-ordered-drain")]
    if _result.is_err() {
        LOST_MEASUREMENTS.store(true, std::sync::atomic::Ordering::Release);
    }
}

pub(crate) fn set_measurements_active(active: bool) {
    #[cfg(feature = "hotpath-ordered-drain")]
    if active {
        LOST_MEASUREMENTS.store(false, std::sync::atomic::Ordering::Release);
    }
    MEASUREMENT_QUEUES.set_active(active);
}

pub(crate) fn sweep_measurements(out: &mut Vec<Measurement>) {
    MEASUREMENT_QUEUES.sweep(out);
}

pub(crate) fn drain_all_measurements(out: &mut Vec<Measurement>) {
    MEASUREMENT_QUEUES.drain_all(out);
}

/// Sweeps until every event published before the call is processed, so an
/// allocation query sees more than one capped sweep. `false` means the query
/// must fail: an event was lost, the registry is poisoned, `timeout` elapsed
/// or shutdown is pending. The snapshot is fixed, so producers that keep
/// publishing cannot stall it.
#[cfg(feature = "hotpath-ordered-drain")]
pub(crate) fn drain_published(
    timeout: Duration,
    swept: &mut Vec<Measurement>,
    mut process: impl FnMut(Measurement),
    shutdown_pending: impl Fn() -> bool,
) -> bool {
    let Some(published) = MEASUREMENT_QUEUES.published_prefix() else {
        return false;
    };
    let deadline = std::time::Instant::now() + timeout;
    while !published.is_processed() {
        if shutdown_pending() || std::time::Instant::now() >= deadline {
            return false;
        }
        sweep_measurements(swept);
        // Unprocessed published events always yield a non-empty sweep unless
        // the registry got poisoned, so an empty one can never catch up.
        if swept.is_empty() {
            return false;
        }
        swept.drain(..).for_each(&mut process);
    }
    !LOST_MEASUREMENTS.load(std::sync::atomic::Ordering::Acquire)
}

#[derive(Debug)]
pub(crate) struct Measurement {
    pub(crate) name: &'static str,
    pub(crate) bytes_total: Option<u64>,
    pub(crate) count_total: Option<u64>,
    pub(crate) duration_ns: Option<u64>,
    pub(crate) elapsed_since_start_ns: u64,
    pub(crate) wrapper: bool,
    pub(crate) tid: Option<u64>,
    pub(crate) result_log: Option<String>,
    /// Axum route scope active when the guard dropped, if any.
    #[cfg(feature = "hotpath-prometheus")]
    pub(crate) route: Option<&'static str>,
}

type LogEntry = (
    Option<u64>,
    Option<u64>,
    Option<u64>,
    Duration,
    Option<u64>,
    Option<String>,
);

#[derive(Debug, Clone)]
pub(crate) struct FunctionStats {
    pub(crate) id: u32,
    pub(crate) name: &'static str,
    pub(crate) count: u64,
    pub(crate) duration_sampled_count: u64,
    bytes_total_hist: Option<Histogram<u64>>,
    count_total_hist: Option<Histogram<u64>>,
    duration_hist: Option<Histogram<u64>>,
    pub(crate) total_bytes_sum: u64,
    pub(crate) total_count_sum: u64,
    pub(crate) total_duration_ns: u64,
    pub(crate) has_data: bool,
    pub(crate) is_async: bool,
    pub(crate) wrapper: bool,
    pub(crate) recent_logs: VecDeque<LogEntry>,
    /// Calls split by the axum route they ran under (Prometheus only).
    #[cfg(feature = "hotpath-prometheus")]
    pub(crate) routes: crate::lib_on::functions::RouteStatsMap,
}

impl FunctionStats {
    const LOW_BYTES: u64 = 1;
    const HIGH_BYTES: u64 = 1_000_000_000; // 1GB
    const LOW_COUNT: u64 = 1;
    const HIGH_COUNT: u64 = 1_000_000_000;
    const LOW_DURATION_NS: u64 = 1;
    const HIGH_DURATION_NS: u64 = crate::lib_on::MAX_DURATION_NS;
    const SIGFIGS: u8 = 3;

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_alloc(
        id: u32,
        name: &'static str,
        bytes_total: Option<u64>,
        count_total: Option<u64>,
        duration_ns: Option<u64>,
        elapsed: Duration,
        wrapper: bool,
        tid: Option<u64>,
        result_log: Option<String>,
    ) -> Self {
        let bytes_total_hist =
            Histogram::<u64>::new_with_bounds(Self::LOW_BYTES, Self::HIGH_BYTES, Self::SIGFIGS)
                .expect("bytes_total histogram init");

        let count_total_hist =
            Histogram::<u64>::new_with_bounds(Self::LOW_COUNT, Self::HIGH_COUNT, Self::SIGFIGS)
                .expect("count_total histogram init");

        let duration_hist = Histogram::<u64>::new_with_bounds(
            Self::LOW_DURATION_NS,
            Self::HIGH_DURATION_NS,
            Self::SIGFIGS,
        )
        .expect("duration histogram init");

        let mut recent_logs = VecDeque::with_capacity(*crate::channels::LOGS_LIMIT);
        recent_logs.push_back((
            bytes_total,
            count_total,
            duration_ns,
            elapsed,
            tid,
            result_log,
        ));

        let mut s = Self {
            id,
            name,
            count: 1,
            duration_sampled_count: 0,
            bytes_total_hist: Some(bytes_total_hist),
            count_total_hist: Some(count_total_hist),
            duration_hist: Some(duration_hist),
            total_bytes_sum: bytes_total.unwrap_or(0),
            total_count_sum: count_total.unwrap_or(0),
            total_duration_ns: 0,
            has_data: true,
            is_async: bytes_total.is_none(),
            wrapper,
            recent_logs,
            #[cfg(feature = "hotpath-prometheus")]
            routes: HashMap::new(),
        };
        s.record_alloc(bytes_total, count_total);
        s.record_duration(duration_ns);
        s
    }

    #[inline]
    fn record_alloc(&mut self, bytes_total: Option<u64>, count_total: Option<u64>) {
        // Zero is always recordable regardless of the histogram's lowest
        // discernible value, so non-allocating calls count toward percentiles.
        if let (Some(ref mut bytes_total_hist), Some(bytes)) =
            (&mut self.bytes_total_hist, bytes_total)
        {
            bytes_total_hist
                .record(bytes.min(Self::HIGH_BYTES))
                .unwrap();
        }
        if let (Some(ref mut count_total_hist), Some(count)) =
            (&mut self.count_total_hist, count_total)
        {
            count_total_hist
                .record(count.min(Self::HIGH_COUNT))
                .unwrap();
        }
    }

    #[inline]
    fn record_duration(&mut self, duration_ns: Option<u64>) {
        let Some(duration_ns) = duration_ns else {
            return;
        };
        self.duration_sampled_count += 1;
        self.total_duration_ns += duration_ns;
        if let Some(ref mut duration_hist) = self.duration_hist {
            let clamped_duration = duration_ns.clamp(Self::LOW_DURATION_NS, Self::HIGH_DURATION_NS);
            duration_hist.record(clamped_duration).unwrap();
        }
    }

    pub(crate) fn update_alloc(
        &mut self,
        bytes_total: Option<u64>,
        count_total: Option<u64>,
        duration_ns: Option<u64>,
        elapsed: Duration,
        tid: Option<u64>,
        result_log: Option<String>,
    ) {
        self.count += 1;
        self.is_async |= bytes_total.is_none();
        self.total_bytes_sum += bytes_total.unwrap_or(0);
        self.total_count_sum += count_total.unwrap_or(0);
        self.record_alloc(bytes_total, count_total);

        self.record_duration(duration_ns);

        if self.recent_logs.len() >= *crate::channels::LOGS_LIMIT {
            self.recent_logs.pop_front();
        }
        self.recent_logs.push_back((
            bytes_total,
            count_total,
            duration_ns,
            elapsed,
            tid,
            result_log,
        ));
    }

    #[inline]
    pub(crate) fn bytes_total_percentile(&self, p: f64) -> u64 {
        if self.count == 0 || self.bytes_total_hist.is_none() {
            return 0;
        }
        let p = p.clamp(0.0, 100.0);
        self.bytes_total_hist
            .as_ref()
            .unwrap()
            .value_at_percentile(p)
    }

    #[inline]
    pub(crate) fn count_total_percentile(&self, p: f64) -> u64 {
        if self.count == 0 || self.count_total_hist.is_none() {
            return 0;
        }
        let p = p.clamp(0.0, 100.0);
        self.count_total_hist
            .as_ref()
            .unwrap()
            .value_at_percentile(p)
    }

    #[inline]
    pub(crate) fn total_bytes(&self) -> u64 {
        self.total_bytes_sum
    }

    #[inline]
    pub(crate) fn avg_bytes(&self) -> u64 {
        if self.count == 0 {
            return 0;
        }
        self.total_bytes_sum / self.count
    }

    #[inline]
    pub(crate) fn total_count(&self) -> u64 {
        self.total_count_sum
    }

    #[inline]
    pub(crate) fn avg_count(&self) -> u64 {
        if self.count == 0 {
            return 0;
        }
        self.total_count_sum / self.count
    }

    #[inline]
    pub(crate) fn duration_percentile(&self, p: f64) -> u64 {
        if self.duration_sampled_count == 0 || self.duration_hist.is_none() {
            return 0;
        }
        let p = p.clamp(0.0, 100.0);
        self.duration_hist.as_ref().unwrap().value_at_percentile(p)
    }

    /// Sparse native-histogram buckets of sampled durations, `(index, count)`
    /// at `schema`, for the Prometheus exporter.
    #[cfg(feature = "hotpath-prometheus")]
    pub(crate) fn native_duration_buckets(&self, schema: i32) -> Vec<(i32, u64)> {
        match self
            .duration_hist
            .as_ref()
            .filter(|_| self.duration_sampled_count > 0)
        {
            Some(hist) => crate::lib_on::native_histograms::native_bucket_counts(
                hist,
                schema,
                crate::lib_on::native_histograms::NANOS_SCALE,
            ),
            None => Vec::new(),
        }
    }

    /// Cumulative classic-bucket counts of sampled durations at or below each
    /// boundary (ns), exact to the histogram's 0.1% resolution.
    #[cfg(feature = "hotpath-prometheus")]
    pub(crate) fn classic_duration_buckets(&self, boundaries: &[u64]) -> Vec<u64> {
        match self
            .duration_hist
            .as_ref()
            .filter(|_| self.duration_sampled_count > 0)
        {
            Some(hist) => {
                crate::lib_on::native_histograms::cumulative_bucket_counts(hist, boundaries)
            }
            None => vec![0; boundaries.len()],
        }
    }

    /// Raw allocation snapshot for the Prometheus exporter, both bucket
    /// projections computed here so no histogram crosses the query channel.
    /// `is_async` entries carry no per-call totals (cross-thread sync guard
    /// drops - the report's N/A percentiles), so they export running totals
    /// only, with empty histogram projections.
    #[cfg(feature = "hotpath-prometheus")]
    pub(crate) fn to_raw_alloc(
        &self,
        schema: i32,
        bytes_boundaries: &[u64],
        allocs_boundaries: &[u64],
    ) -> crate::lib_on::functions::RawFunctionAlloc {
        use crate::lib_on::native_histograms::{
            classic_buckets_opt, native_buckets_opt, UNIT_SCALE,
        };

        let bytes_hist = self.bytes_total_hist.as_ref().filter(|_| !self.is_async);
        let count_hist = self.count_total_hist.as_ref().filter(|_| !self.is_async);
        crate::lib_on::functions::RawFunctionAlloc {
            name: self.name,
            total_bytes: self.total_bytes_sum,
            total_allocs: self.total_count_sum,
            bytes_sample_count: bytes_hist.map_or(0, |h| h.len()),
            allocs_sample_count: count_hist.map_or(0, |h| h.len()),
            bytes_native: native_buckets_opt(bytes_hist, true, schema, UNIT_SCALE),
            bytes_classic: classic_buckets_opt(bytes_hist, true, bytes_boundaries),
            allocs_native: native_buckets_opt(count_hist, true, schema, UNIT_SCALE),
            allocs_classic: classic_buckets_opt(count_hist, true, allocs_boundaries),
            bytes_zero_count: bytes_hist.map_or(0, |h| h.count_at(0)),
            allocs_zero_count: count_hist.map_or(0, |h| h.count_at(0)),
            routes: crate::lib_on::functions::raw_routes(&self.routes),
        }
    }

    #[cfg(feature = "hotpath-prometheus")]
    #[inline]
    fn record_route(
        &mut self,
        route: Option<&'static str>,
        duration_ns: Option<u64>,
        bytes_total: Option<u64>,
        count_total: Option<u64>,
    ) {
        if let Some(route) = route {
            self.routes
                .entry(route)
                .or_default()
                .record(duration_ns, bytes_total, count_total);
        }
    }

    #[inline]
    pub(crate) fn avg_duration_ns(&self) -> u64 {
        self.total_duration_ns
            .checked_div(self.duration_sampled_count)
            .unwrap_or(0)
    }

    /// Exact when every call was timed, extrapolated (`avg * count`) under time sampling.
    #[inline]
    pub(crate) fn display_total_duration_ns(&self) -> u64 {
        if self.duration_sampled_count == self.count {
            self.total_duration_ns
        } else {
            self.avg_duration_ns() * self.count
        }
    }
}

pub(crate) struct FunctionsState {
    pub shutdown_tx: Option<crossbeam_channel::Sender<()>>,
    pub completion_rx: Option<Mutex<Receiver<HashMap<u32, FunctionStats>>>>,

    pub start_time: Instant,
    pub caller_name: &'static str,
    pub percentiles: Vec<f64>,
    pub limit: usize,
}

pub(crate) fn process_measurement(
    stats: &mut HashMap<u32, FunctionStats>,
    name_to_id: &mut HashMap<&'static str, u32>,
    m: Measurement,
) {
    let elapsed = Duration::from_nanos(m.elapsed_since_start_ns);
    if let Some(&id) = name_to_id.get(m.name) {
        if let Some(s) = stats.get_mut(&id) {
            #[cfg(feature = "hotpath-prometheus")]
            s.record_route(m.route, m.duration_ns, m.bytes_total, m.count_total);
            s.update_alloc(
                m.bytes_total,
                m.count_total,
                m.duration_ns,
                elapsed,
                m.tid,
                m.result_log,
            );
        }
    } else {
        let id = crate::functions::next_function_id();
        name_to_id.insert(m.name, id);
        #[cfg_attr(not(feature = "hotpath-prometheus"), allow(unused_mut))]
        let mut s = FunctionStats::new_alloc(
            id,
            m.name,
            m.bytes_total,
            m.count_total,
            m.duration_ns,
            elapsed,
            m.wrapper,
            m.tid,
            m.result_log,
        );
        #[cfg(feature = "hotpath-prometheus")]
        s.record_route(m.route, m.duration_ns, m.bytes_total, m.count_total);
        stats.insert(id, s);
    }
}

use crate::lib_on::functions::FUNCTIONS_STATE;

#[allow(clippy::too_many_arguments)]
pub(crate) fn send_alloc_measurement(
    name: &'static str,
    bytes_total: Option<u64>,
    count_total: Option<u64>,
    duration_ns: Option<u64>,
    elapsed_since_start_ns: u64,
    wrapper: bool,
    tid: Option<u64>,
) {
    send_alloc_measurement_with_log(
        name,
        bytes_total,
        count_total,
        duration_ns,
        elapsed_since_start_ns,
        wrapper,
        tid,
        None,
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn send_alloc_measurement_with_log(
    name: &'static str,
    bytes_total: Option<u64>,
    count_total: Option<u64>,
    duration_ns: Option<u64>,
    elapsed_since_start_ns: u64,
    wrapper: bool,
    tid: Option<u64>,
    result_log: Option<String>,
) {
    if FUNCTIONS_STATE.get().is_none() {
        return;
    }

    add_measurement(Measurement {
        name,
        bytes_total,
        count_total,
        duration_ns,
        elapsed_since_start_ns,
        wrapper,
        tid,
        result_log,
        #[cfg(feature = "hotpath-prometheus")]
        route: crate::lib_on::caller_stack::current_route(),
    });
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::lib_on::functions::alloc::state::FunctionStats;

    #[test]
    fn zero_duration_samples_land_in_histogram() {
        let mut stats = FunctionStats::new_alloc(
            1,
            "f",
            Some(0),
            Some(0),
            Some(0),
            Duration::ZERO,
            false,
            None,
            None,
        );
        stats.update_alloc(Some(0), Some(0), Some(0), Duration::ZERO, None, None);
        stats.update_alloc(Some(0), Some(0), Some(500), Duration::ZERO, None, None);

        assert_eq!(stats.duration_sampled_count, 3);
        assert_eq!(stats.duration_hist.as_ref().unwrap().len(), 3);
    }

    #[test]
    fn zero_alloc_samples_land_in_histograms() {
        let mut stats = FunctionStats::new_alloc(
            1,
            "f",
            Some(0),
            Some(0),
            Some(10),
            Duration::ZERO,
            false,
            None,
            None,
        );
        stats.update_alloc(Some(0), Some(0), Some(10), Duration::ZERO, None, None);
        stats.update_alloc(Some(0), Some(0), Some(10), Duration::ZERO, None, None);
        stats.update_alloc(Some(4096), Some(2), Some(10), Duration::ZERO, None, None);

        assert_eq!(stats.count, 4);
        assert_eq!(stats.bytes_total_hist.as_ref().unwrap().len(), 4);
        assert_eq!(stats.count_total_hist.as_ref().unwrap().len(), 4);
        assert_eq!(stats.bytes_total_percentile(50.0), 0);
        assert_eq!(stats.count_total_percentile(50.0), 0);
        assert!(stats.bytes_total_percentile(100.0) >= 4096);
        assert_eq!(stats.count_total_percentile(100.0), 2);
    }
}

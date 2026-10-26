use crate::instant::Instant;
use arc_swap::ArcSwapOption;
use crossbeam_channel::{bounded, unbounded, Select};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};
use std::thread;

pub(crate) static CONFIGURED_PERCENTILES: std::sync::OnceLock<Vec<f64>> =
    std::sync::OnceLock::new();

/// Upper bound on configured percentiles. Every percentile adds a column to each
/// report table and a full histogram scan per entry on every metrics request, so
/// the ceiling is set by what a table can legibly render. Kept in sync with
/// `MAX_PERCENTILES` in `hotpath-macros`.
pub(crate) const MAX_PERCENTILES: usize = 10;

/// Sorts and deduplicates the configured percentiles. Panics on values outside
/// `0..=100` (NaN included) and on more than [`MAX_PERCENTILES`] distinct values.
pub(crate) fn normalize_percentiles(percentiles: &[f64]) -> Vec<f64> {
    for &p in percentiles {
        if !(0.0..=100.0).contains(&p) {
            panic!("Invalid percentile {p} (must be 0..=100)");
        }
    }

    let mut normalized = percentiles.to_vec();
    normalized.sort_by(|a, b| a.partial_cmp(b).expect("percentiles are not NaN"));
    normalized.dedup();

    if normalized.len() > MAX_PERCENTILES {
        panic!(
            "Too many percentiles: {} (max {MAX_PERCENTILES})",
            normalized.len()
        );
    }

    normalized
}

pub(crate) fn configured_percentiles() -> Vec<f64> {
    CONFIGURED_PERCENTILES
        .get()
        .cloned()
        .unwrap_or_else(|| vec![95.0])
}

/// Parses the `HOTPATH_USER_METADATA` grammar: comma-separated `key=value`
/// pairs, split on the first `=` so values may contain `=` (but not `,`).
/// Keys and values are trimmed. Errors on a segment without `=`, an empty
/// segment (e.g. a trailing comma), an empty key, or a repeated key.
pub(crate) fn parse_user_metadata(raw: &str) -> Result<BTreeMap<String, String>, String> {
    let mut metadata = BTreeMap::new();
    if raw.trim().is_empty() {
        return Ok(metadata);
    }

    for segment in raw.split(',') {
        let segment = segment.trim();
        let Some((key, value)) = segment.split_once('=') else {
            return Err(format!(
                "invalid HOTPATH_USER_METADATA: expected 'key=value', got '{segment}'"
            ));
        };
        let key = key.trim();
        if key.is_empty() {
            return Err(format!(
                "invalid HOTPATH_USER_METADATA: empty key in '{segment}'"
            ));
        }
        if metadata
            .insert(key.to_string(), value.trim().to_string())
            .is_some()
        {
            return Err(format!(
                "invalid HOTPATH_USER_METADATA: duplicate key '{key}'"
            ));
        }
    }

    Ok(metadata)
}

/// Merges `HOTPATH_USER_METADATA` over the builder-supplied pairs; env values
/// win per key. Panics on an unparsable env value.
fn resolve_user_metadata(builder: HashMap<String, String>) -> BTreeMap<String, String> {
    let mut metadata: BTreeMap<String, String> = builder.into_iter().collect();
    let raw = match std::env::var("HOTPATH_USER_METADATA") {
        Ok(raw) => raw,
        Err(std::env::VarError::NotPresent) => return metadata,
        Err(std::env::VarError::NotUnicode(_)) => {
            panic!("invalid HOTPATH_USER_METADATA: value is not valid UTF-8")
        }
    };
    match parse_user_metadata(&raw) {
        Ok(from_env) => metadata.extend(from_env),
        Err(err) => panic!("{err}"),
    }
    metadata
}

const DEFAULT_DRAIN_INTERVAL_MS: u64 = 50;
/// Interval between worker sweeps of the per-thread event queues. Lowering it
/// bounds queue growth for high-traffic apps at the cost of more worker wakeups.
pub(crate) static DRAIN_INTERVAL_MS: LazyLock<u64> = LazyLock::new(|| {
    // `HOTPATH_DRAIN_INTERVAL` is the legacy name, still honored when the new one is unset.
    std::env::var("HOTPATH_DRAIN_INTERVAL_MS")
        .or_else(|_| std::env::var("HOTPATH_DRAIN_INTERVAL"))
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_DRAIN_INTERVAL_MS)
});

const DEFAULT_LOGS_LIMIT: usize = 50;
pub(crate) static LOGS_LIMIT: LazyLock<usize> = LazyLock::new(|| {
    std::env::var("HOTPATH_LOGS_LIMIT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_LOGS_LIMIT)
});

/// Maximum number of distinct entries kept per runtime-keyed subsystem (server
/// routes, outbound HTTP endpoints, SQL queries). Once reached, new keys are
/// folded into a single [`OVERFLOW_ENTRY`] bucket so attacker- or data-driven
/// cardinality (unmatched 404 paths, dynamic SQL) cannot grow memory without
/// bound. Compile-time keyed subsystems (functions, channels, ...) need no cap.
const DEFAULT_ENTRIES_LIMIT: usize = 1000;
pub(crate) static ENTRIES_LIMIT: LazyLock<usize> = LazyLock::new(|| {
    std::env::var("HOTPATH_ENTRIES_LIMIT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_ENTRIES_LIMIT)
});

/// Name of the bucket that absorbs entries beyond [`ENTRIES_LIMIT`].
pub(crate) const OVERFLOW_ENTRY: &str = "<other>";

/// Path part of the server-route bucket that absorbs unmatched requests
/// ending in an error status (internet-scanner probes like `GET /.env`);
/// the method prefix is preserved, e.g. `GET <unmatched>`.
pub(crate) const UNMATCHED_ENTRY: &str = "<unmatched>";

/// Returns `key` when `map` already holds it or still has room under
/// [`ENTRIES_LIMIT`]; otherwise returns the overflow key from `overflow`. One
/// slot is reserved for the overflow bucket, so the map never exceeds the
/// limit. Callers must produce a single fixed overflow key per map.
pub(crate) fn bounded_key<K, V>(
    map: &std::collections::HashMap<K, V>,
    key: K,
    overflow: impl FnOnce() -> K,
) -> K
where
    K: Eq + std::hash::Hash,
{
    bounded_key_with_limit(map, *ENTRIES_LIMIT, key, overflow)
}

fn bounded_key_with_limit<K, V>(
    map: &std::collections::HashMap<K, V>,
    limit: usize,
    key: K,
    overflow: impl FnOnce() -> K,
) -> K
where
    K: Eq + std::hash::Hash,
{
    if map.contains_key(&key) || map.len() + 1 < limit {
        key
    } else {
        overflow()
    }
}

use std::io::Write;

use crate::json::{JsonFunctionsList, JsonReport};
use crate::metrics_server::METRICS_SERVER_PORT;
use crate::output::{FunctionLog, FunctionLogsList};
use crate::output_on::{
    display_functions_table_to, display_no_measurements_message_to, resolve_output_path,
    write_report_footer, write_report_header, write_user_metadata_table, OutputDestination,
};

#[cfg(feature = "hotpath-prometheus")]
use crate::functions::RawFunctionTiming;
use crate::functions::{FunctionsQuery, Measurement, FUNCTIONS_QUERY_TX, FUNCTIONS_STATE};
use crate::lib_on::report;
use crate::lib_on::report::apply_limit;
use crate::shared::{Section, SectionsMode};

#[cfg(feature = "hotpath-cpu")]
use crate::dev_logging::{info, warn};
use crate::functions::FunctionStatsConfig;

cfg_if::cfg_if! {
    if #[cfg(feature = "hotpath-alloc")] {
        use crate::functions::alloc::{
            report::{build_functions_list_alloc, build_functions_list_timing},
            state::{FunctionStats, FunctionsState, drain_all_measurements, process_measurement, set_measurements_active, sweep_measurements},
        };
    } else {
        use crate::functions::timing::{
            report::build_functions_list,
            state::{FunctionStats, FunctionsState, drain_all_measurements, process_measurement, set_measurements_active, sweep_measurements},
        };
    }
}

use crate::functions::MeasurementGuardSync;
use crate::Format;

/// Builder for [`HotpathGuard`] - a programmatic alternative to the
/// `#[hotpath::main]` macro for configuring and initializing the profiler.
///
/// Dropping the resulting [`HotpathGuard`] generates the profiling report, so
/// the guard must be held alive for the duration you want to profile.
///
/// # Example
///
/// ```rust,no_run
/// use hotpath::{HotpathGuardBuilder, Format, Section};
///
/// let _guard = HotpathGuardBuilder::new("main")
///     .percentiles(&[50.0, 95.0, 99.9])
///     .functions_limit(20)
///     .channels_limit(5)
///     .format(Format::JsonPretty)
///     .output_path("report.json")
///     .sections(vec![Section::FunctionsTiming, Section::Channels])
///     .build();
/// ```
#[must_use = "builder is discarded without creating a guard"]
pub struct HotpathGuardBuilder {
    caller_name: &'static str,
    percentiles: Vec<f64>,
    format: Format,
    functions_limit: usize,
    channels_limit: usize,
    streams_limit: usize,
    futures_limit: usize,
    rw_locks_limit: usize,
    mutexes_limit: usize,
    sql_limit: usize,
    http_limit: usize,
    server_limit: usize,
    io_limit: usize,
    threads_limit: usize,
    #[cfg(feature = "axum-0-8")]
    route_scope: bool,
    output_path: Option<PathBuf>,
    sections_mode: Option<SectionsMode>,
    before_shutdown: Option<Box<dyn FnOnce() + Send + Sync>>,
    time_sampling: crate::lib_on::sampling::TimeSamplingConfig,
    user_metadata: HashMap<String, String>,
}

impl HotpathGuardBuilder {
    /// Creates a new builder.
    ///
    /// `caller_name` identifies the top-level wrapper function in reports
    /// (typically `"main"`).
    ///
    /// # Defaults
    ///
    /// | Option | Default |
    /// |---|---|
    /// | `percentiles` | `[95]` |
    /// | `format` | [`Format::Table`] |
    /// | `functions_limit` | `15` |
    /// | `channels_limit` | `0` (unlimited) |
    /// | `streams_limit` | `0` (unlimited) |
    /// | `futures_limit` | `0` (unlimited) |
    /// | `threads_limit` | `5` |
    /// | `sections` | auto: functions + threads, plus every instrumented section with data |
    pub fn new(caller_name: &'static str) -> Self {
        Self {
            caller_name,
            percentiles: vec![95.0],
            format: Format::Table,
            functions_limit: 15,
            channels_limit: 0,
            streams_limit: 0,
            futures_limit: 0,
            rw_locks_limit: 0,
            mutexes_limit: 0,
            sql_limit: 0,
            http_limit: 0,
            server_limit: 0,
            io_limit: 0,
            threads_limit: 5,
            #[cfg(feature = "axum-0-8")]
            route_scope: true,
            output_path: None,
            sections_mode: None,
            before_shutdown: None,
            time_sampling: crate::lib_on::sampling::TimeSamplingConfig::default(),
            user_metadata: HashMap::new(),
        }
    }

    /// Sets which latency percentiles to compute (e.g. `&[50.0, 95.0, 99.9]`).
    /// Values are sorted and deduplicated, and at most 10 distinct percentiles
    /// are allowed - each one adds a column to every report table.
    pub fn percentiles(mut self, percentiles: &[f64]) -> Self {
        self.percentiles = percentiles.to_vec();
        self
    }

    /// Sets the maximum number of items shown in every report section (except debug).
    /// Set to `0` for unlimited. Per-resource limits (e.g. `functions_limit`)
    /// called after this method will override the global value for that section.
    pub fn limit(mut self, limit: usize) -> Self {
        self.functions_limit = limit;
        self.channels_limit = limit;
        self.streams_limit = limit;
        self.futures_limit = limit;
        self.rw_locks_limit = limit;
        self.mutexes_limit = limit;
        self.sql_limit = limit;
        self.http_limit = limit;
        self.server_limit = limit;
        self.io_limit = limit;
        self.threads_limit = limit;
        self
    }

    /// Maximum number of functions shown in the report. Set to `0` for unlimited.
    pub fn functions_limit(mut self, limit: usize) -> Self {
        self.functions_limit = limit;
        self
    }

    /// Maximum number of channels shown in the report. Set to `0` for unlimited.
    pub fn channels_limit(mut self, limit: usize) -> Self {
        self.channels_limit = limit;
        self
    }

    /// Maximum number of streams shown in the report. Set to `0` for unlimited.
    pub fn streams_limit(mut self, limit: usize) -> Self {
        self.streams_limit = limit;
        self
    }

    /// Maximum number of futures shown in the report. Set to `0` for unlimited.
    pub fn futures_limit(mut self, limit: usize) -> Self {
        self.futures_limit = limit;
        self
    }

    /// Maximum number of rw_locks shown in the report. Set to `0` for unlimited.
    pub fn rw_locks_limit(mut self, limit: usize) -> Self {
        self.rw_locks_limit = limit;
        self
    }

    /// Maximum number of mutexes shown in the report. Set to `0` for unlimited.
    pub fn mutexes_limit(mut self, limit: usize) -> Self {
        self.mutexes_limit = limit;
        self
    }

    /// Maximum number of SQL queries shown in the report. Set to `0` for unlimited.
    pub fn sql_limit(mut self, limit: usize) -> Self {
        self.sql_limit = limit;
        self
    }

    /// Maximum number of HTTP endpoints shown in the report. Set to `0` for unlimited.
    pub fn http_limit(mut self, limit: usize) -> Self {
        self.http_limit = limit;
        self
    }

    /// Maximum number of server routes shown in the report. Set to `0` for unlimited.
    pub fn server_limit(mut self, limit: usize) -> Self {
        self.server_limit = limit;
        self
    }

    /// Whether SQL queries and outbound HTTP requests issued while handling an
    /// axum request are attributed to the matched route (the `route` field /
    /// "Route" column). Enabled by default; `HOTPATH_ROUTE_SCOPE=0` overrides.
    #[cfg(feature = "axum-0-8")]
    pub fn route_scope(mut self, enabled: bool) -> Self {
        self.route_scope = enabled;
        self
    }

    /// Maximum number of I/O wrappers shown in the report. Set to `0` for unlimited.
    pub fn io_limit(mut self, limit: usize) -> Self {
        self.io_limit = limit;
        self
    }

    /// Maximum number of threads shown in the report. Set to `0` for unlimited.
    pub fn threads_limit(mut self, limit: usize) -> Self {
        self.threads_limit = limit;
        self
    }

    /// Sets the fraction of calls whose duration is measured, in `[0.0, 1.0]`
    /// (e.g. `0.1` times about 1 in 10 calls, `0.0` keeps exact counts but no
    /// durations). Applies to functions, mutexes, rw_locks, futures, and wrap
    /// channels; per-resource setters override it. Env vars
    /// (`HOTPATH_TIME_SAMPLING_RATE` and per-resource variants) take precedence.
    /// Under `hotpath-alloc`, function durations respect the rate while
    /// allocation metrics stay exact. Under `hotpath-callgrind` running in
    /// Valgrind, every rate is forced to `0.0`.
    pub fn time_sampling_rate(mut self, rate: f64) -> Self {
        self.time_sampling.global = Some(rate);
        self
    }

    /// Fraction of function calls whose duration is measured. Overrides
    /// [`time_sampling_rate`](Self::time_sampling_rate) for functions.
    pub fn functions_time_sampling_rate(mut self, rate: f64) -> Self {
        self.time_sampling.functions = Some(rate);
        self
    }

    /// Fraction of mutex acquisitions whose wait/acquire time is measured.
    pub fn mutexes_time_sampling_rate(mut self, rate: f64) -> Self {
        self.time_sampling.mutexes = Some(rate);
        self
    }

    /// Fraction of RwLock acquisitions whose wait/acquire time is measured.
    pub fn rw_locks_time_sampling_rate(mut self, rate: f64) -> Self {
        self.time_sampling.rw_locks = Some(rate);
        self
    }

    /// Fraction of future calls whose poll durations are measured; the
    /// decision is made once per call, so a call has all polls timed or none.
    pub fn futures_time_sampling_rate(mut self, rate: f64) -> Self {
        self.time_sampling.futures = Some(rate);
        self
    }

    /// Fraction of wrap-channel messages whose send->receive latency is measured.
    pub fn channels_time_sampling_rate(mut self, rate: f64) -> Self {
        self.time_sampling.channels = Some(rate);
        self
    }

    /// Fraction of I/O operations whose duration is measured.
    pub fn io_time_sampling_rate(mut self, rate: f64) -> Self {
        self.time_sampling.io = Some(rate);
        self
    }

    /// Sets the output format. Overridden at runtime by `HOTPATH_OUTPUT_FORMAT` env var.
    pub fn format(mut self, format: Format) -> Self {
        self.format = format;
        self
    }

    /// Writes the report to a file instead of stdout. Overridden by `HOTPATH_OUTPUT_PATH` env var.
    pub fn output_path(mut self, path: impl AsRef<std::path::Path>) -> Self {
        self.output_path = Some(resolve_output_path(path));
        self
    }

    /// Chooses exactly which report sections to include; sections not listed
    /// are never shown, even if they have data. Overridden by the
    /// `HOTPATH_REPORT` env var. Without this call the report is in auto mode:
    /// function and thread sections plus every instrumented section with data.
    pub fn sections(mut self, sections: Vec<Section>) -> Self {
        self.sections_mode = Some(SectionsMode::Explicit(sections));
        self
    }

    /// Hides the given sections from the auto report while keeping every other
    /// section auto-included. Mutually exclusive with
    /// [`sections`](Self::sections) - the last call wins. Overridden by the
    /// `HOTPATH_REPORT` env var.
    pub fn sections_exclude(mut self, sections: Vec<Section>) -> Self {
        self.sections_mode = Some(SectionsMode::Auto {
            include: Vec::new(),
            exclude: sections,
        });
        self
    }

    /// Configures report sections from a spec string using the same grammar as
    /// the `HOTPATH_REPORT` env var (which still takes precedence): `"all"`,
    /// `"auto"`, an exact list like `"channels,sql"`, or auto with exclusions
    /// like `"auto,-threads"` / `"-threads"`.
    pub fn report(mut self, spec: &str) -> Self {
        self.sections_mode = Some(SectionsMode::parse(spec));
        self
    }

    /// Registers a callback that runs just before the guard is dropped and the report is generated.
    pub fn before_shutdown(mut self, f: impl FnOnce() + Send + Sync + 'static) -> Self {
        self.before_shutdown = Some(Box::new(f));
        self
    }

    /// Attaches free-form key/value metadata to the report: rendered as a
    /// `user_metadata` table in table output and as a top-level
    /// `user_metadata` object in JSON output. Pairs from the
    /// `HOTPATH_USER_METADATA` env var (`key1=val1,key2=val2`) are merged on
    /// top and win for keys present in both.
    pub fn user_metadata(mut self, metadata: HashMap<String, String>) -> Self {
        self.user_metadata = metadata;
        self
    }

    fn resolve_sections_mode(&self) -> SectionsMode {
        if let Some(env_mode) = SectionsMode::from_env() {
            return env_mode;
        }

        self.sections_mode.clone().unwrap_or_default()
    }

    /// Consumes the builder and initializes the profiler, returning a [`HotpathGuard`].
    ///
    /// # Panics
    ///
    /// Panics if another `HotpathGuard` is already alive, if `HOTPATH_FOCUS`
    /// holds an invalid regex pattern, if a configured percentile falls outside
    /// `0..=100`, if more than 10 distinct percentiles are configured, or if
    /// `HOTPATH_USER_METADATA` is not a comma-separated list of `key=value`
    /// pairs with unique, non-empty keys.
    pub fn build(self) -> HotpathGuard {
        #[cfg(feature = "dev")]
        crate::dev_logging::init_logging();

        crate::lib_on::functions::init_focus_filter();

        crate::lib_on::sampling::init_time_sampling_rate(&self.time_sampling);

        #[cfg(feature = "axum-0-8")]
        {
            let enabled = match std::env::var("HOTPATH_ROUTE_SCOPE") {
                Ok(v) => !(v == "0" || v.eq_ignore_ascii_case("false")),
                Err(_) => self.route_scope,
            };
            crate::lib_on::caller_stack::set_route_scope(enabled);
        }

        let sections_mode = self.resolve_sections_mode();
        let percentiles = normalize_percentiles(&self.percentiles);
        let user_metadata = resolve_user_metadata(self.user_metadata);

        HotpathGuard::new(
            self.caller_name,
            &percentiles,
            self.functions_limit,
            self.format,
            self.output_path,
            sections_mode,
            self.before_shutdown,
            self.channels_limit,
            self.streams_limit,
            self.futures_limit,
            self.rw_locks_limit,
            self.mutexes_limit,
            self.sql_limit,
            self.http_limit,
            self.server_limit,
            self.io_limit,
            self.threads_limit,
            user_metadata,
        )
    }

    /// Builds the guard and moves it to a background thread that keeps it alive.
    ///
    /// If `duration` is non-zero (or overridden by `HOTPATH_SHUTDOWN_MS`), the
    /// process exits after that timeout and the report is printed. Otherwise the
    /// guard lives until the process exits.
    pub fn build_with_shutdown(self, duration: std::time::Duration) {
        let guard = self.build();
        if let Some(timeout) =
            crate::shared::resolve_timeout_duration(duration, "HOTPATH_SHUTDOWN_MS")
        {
            thread::spawn(move || {
                thread::sleep(timeout);
                drop(guard);
                std::process::exit(0);
            });
        } else {
            thread::spawn(move || {
                let _guard = guard;
                loop {
                    thread::park();
                }
            });
        }
    }
}

/// RAII guard that owns the profiler lifetime.
///
/// When dropped, it shuts down background workers, collects all measurements,
/// and writes the profiling report. Create one via [`HotpathGuardBuilder`].
#[must_use = "guard is dropped immediately without generating a report"]
pub struct HotpathGuard {
    state: Arc<crate::lib_on::MetaRwLock<FunctionsState>>,
    format: Format,
    wrapper_guard: Option<MeasurementGuardSync>,
    output_path: Option<PathBuf>,
    sections_mode: SectionsMode,
    start_time: Instant,
    before_shutdown: Option<Box<dyn FnOnce() + Send + Sync>>,
    channels_limit: usize,
    streams_limit: usize,
    futures_limit: usize,
    rw_locks_limit: usize,
    mutexes_limit: usize,
    sql_limit: usize,
    http_limit: usize,
    server_limit: usize,
    io_limit: usize,
    threads_limit: usize,
    user_metadata: BTreeMap<String, String>,
    #[cfg(feature = "hotpath-meta")]
    _meta_guard: Option<hotpath_meta::HotpathGuard>,
}

impl HotpathGuard {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        caller_name: &'static str,
        percentiles: &[f64],
        limit: usize,
        format: Format,
        output_path: Option<PathBuf>,
        sections_mode: SectionsMode,
        before_shutdown: Option<Box<dyn FnOnce() + Send + Sync>>,
        channels_limit: usize,
        streams_limit: usize,
        futures_limit: usize,
        rw_locks_limit: usize,
        mutexes_limit: usize,
        sql_limit: usize,
        http_limit: usize,
        server_limit: usize,
        io_limit: usize,
        threads_limit: usize,
        user_metadata: BTreeMap<String, String>,
    ) -> Self {
        let _suspend = crate::lib_on::SuspendAllocTracking::new();

        let percentiles = percentiles.to_vec();
        let _ = CONFIGURED_PERCENTILES.set(percentiles.clone());

        let arc_swap = FUNCTIONS_STATE.get_or_init(|| ArcSwapOption::from(None));

        if arc_swap.load().is_some() {
            panic!("More than one _hotpath guard cannot be alive at the same time.");
        }

        let (query_tx, query_rx) = unbounded::<FunctionsQuery>();
        let (shutdown_tx, shutdown_rx) = bounded::<()>(1);
        let (completion_tx, completion_rx) = bounded::<HashMap<u32, FunctionStats>>(1);
        let _ = FUNCTIONS_QUERY_TX.set(query_tx);
        let start_time = Instant::now();

        let state_arc = Arc::new(crate::lib_on::meta_rw_lock!(
            "functions_state",
            FunctionsState {
                shutdown_tx: Some(shutdown_tx),
                completion_rx: Some(Mutex::new(completion_rx)),
                start_time,
                caller_name,
                percentiles: percentiles.clone(),
                limit,
            },
        ));

        let worker_start_time = start_time;
        let worker_percentiles = percentiles.clone();
        let worker_caller_name = caller_name;
        let worker_limit = limit;
        thread::Builder::new()
            .name("hp-functions".into())
            .spawn(move || {
                let _suspend = crate::lib_on::SuspendAllocTracking::new();

                let mut local_stats = HashMap::<u32, FunctionStats>::new();
                let mut name_to_id = HashMap::<&'static str, u32>::new();
                #[cfg(feature = "hotpath-cpu")]
                if *crate::functions::cpu::CPU_INCLUSIVE {
                    name_to_id.insert(worker_caller_name, 0);
                }

                // Measurements live in per-thread SPSC queues (see `batch.rs`); this
                // worker is their single consumer and sweeps them on a fixed tick.
                // Control channels (shutdown, queries) wake the worker early via
                // `Select::ready_timeout` and are handled right after a sweep, so
                // both shutdown and queries always see freshly drained data.
                let mut select = Select::new();
                let _shutdown_idx = select.recv(&shutdown_rx);
                let _query_idx = select.recv(&query_rx);
                let flush_interval = std::time::Duration::from_millis(*DRAIN_INTERVAL_MS);
                let mut swept: Vec<Measurement> = Vec::new();

                loop {
                    let _ = select.ready_timeout(flush_interval);

                    // Uncapped drain on shutdown: producers are already stopped,
                    // there is no next tick to pick up capped-sweep leftovers.
                    if shutdown_rx.try_recv().is_ok() {
                        drain_all_measurements(&mut swept);
                        for measurement in swept.drain(..) {
                            process_measurement(&mut local_stats, &mut name_to_id, measurement);
                        }
                        break;
                    }

                    sweep_measurements(&mut swept);
                    for measurement in swept.drain(..) {
                        process_measurement(&mut local_stats, &mut name_to_id, measurement);
                    }

                    while let Ok(query_request) = query_rx.try_recv() {
                        {
                            let config = FunctionStatsConfig {
                                    total_elapsed: worker_start_time.elapsed(),
                                    percentiles: worker_percentiles.clone(),
                                    caller_name: worker_caller_name,
                                    limit: worker_limit,
                                    cloud: false,
                                };
                                let current_elapsed_ns = config.total_elapsed.as_nanos() as u64;

                                match query_request {
                                    FunctionsQuery::Alloc(response_tx) => {
                                        cfg_if::cfg_if! {
                                            if #[cfg(feature = "hotpath-alloc")] {
                                                let formatted = build_functions_list_alloc(
                                                    &local_stats, &config, current_elapsed_ns,
                                                );
                                                let _ = response_tx.send(Some(formatted));
                                            } else {
                                                let _ = response_tx.send(None);
                                            }
                                        }
                                    }
                                    FunctionsQuery::Timing(response_tx) => {
                                        cfg_if::cfg_if! {
                                            if #[cfg(feature = "hotpath-alloc")] {
                                                let formatted = build_functions_list_timing(
                                                    &local_stats, &config, current_elapsed_ns,
                                                );
                                            } else {
                                                let formatted = build_functions_list(
                                                    &local_stats, &config, current_elapsed_ns,
                                                );
                                            }
                                        }
                                        let _ = response_tx.send(formatted);
                                    }
                                    #[cfg(feature = "hotpath-prometheus")]
                                    FunctionsQuery::TimingRaw(response_tx) => {
                                        let exclude_wrapper = *crate::functions::EXCLUDE_WRAPPER;
                                        let schema = crate::prometheus_server::NATIVE_SCHEMA;
                                        let mut raw: Vec<RawFunctionTiming> = local_stats
                                            .values()
                                            .filter(|s| s.has_data && !(exclude_wrapper && s.wrapper))
                                            .map(|s| {
                                                cfg_if::cfg_if! {
                                                    if #[cfg(feature = "hotpath-alloc")] {
                                                        let sampled_count = s.duration_sampled_count;
                                                    } else {
                                                        let sampled_count = s.sampled_count;
                                                    }
                                                }
                                                RawFunctionTiming {
                                                    name: s.name,
                                                    count: s.count,
                                                    sampled_count,
                                                    total_duration_ns: s.total_duration_ns,
                                                    native_buckets: s.native_duration_buckets(schema),
                                                    bucket_counts: s.classic_duration_buckets(
                                                        &crate::prometheus_server::FAST_LADDER_NS,
                                                    ),
                                                    routes: crate::functions::raw_routes(&s.routes),
                                                }
                                            })
                                            .collect();
                                        raw.sort_by(|a, b| a.name.cmp(b.name));
                                        let _ = response_tx.send(raw);
                                    }
                                    #[cfg(feature = "hotpath-prometheus")]
                                    FunctionsQuery::AllocRaw(response_tx) => {
                                        cfg_if::cfg_if! {
                                            if #[cfg(feature = "hotpath-alloc")] {
                                                let exclude_wrapper = *crate::functions::EXCLUDE_WRAPPER;
                                                let schema = crate::prometheus_server::NATIVE_SCHEMA;
                                                let mut raw: Vec<crate::functions::RawFunctionAlloc> = local_stats
                                                    .values()
                                                    .filter(|s| s.has_data && !(exclude_wrapper && s.wrapper))
                                                    .map(|s| s.to_raw_alloc(
                                                        schema,
                                                        crate::prometheus_server::ALLOC_LADDER_BYTES,
                                                        crate::prometheus_server::ALLOC_LADDER_COUNT,
                                                    ))
                                                    .collect();
                                                raw.sort_by(|a, b| a.name.cmp(b.name));
                                                let _ = response_tx.send(Some(raw));
                                            } else {
                                                let _ = response_tx.send(None);
                                            }
                                        }
                                    }
                                    #[cfg(feature = "hotpath-cpu")]
                                    FunctionsQuery::NamesAndIds(response_tx) => {
                                        let map: HashMap<&'static str, u32> =
                                            name_to_id
                                                .iter()
                                                .map(|(name, id)| (*name, *id))
                                                .collect();
                                        let _ = response_tx.send(map);
                                    }
                                    FunctionsQuery::LogsTiming { function_id, response_tx } => {
                                        let response = local_stats.get(&function_id)
                                            .map(|stats| {
                                                cfg_if::cfg_if! {
                                                    if #[cfg(feature = "hotpath-alloc")] {
                                                        let logs: Vec<FunctionLog> = stats.recent_logs
                                                            .iter()
                                                            .rev()
                                                            .map(|(_bytes, _count, duration_ns, elapsed, tid, result_log)| FunctionLog {
                                                                value: *duration_ns,
                                                                elapsed_nanos: elapsed.as_nanos() as u64,
                                                                alloc_count: None,
                                                                tid: *tid,
                                                                result: result_log.clone(),
                                                            })
                                                            .collect();
                                                    } else {
                                                        let logs: Vec<FunctionLog> = stats.recent_logs
                                                            .iter()
                                                            .rev()
                                                            .map(|(duration_ns, elapsed, tid, result_log)| FunctionLog {
                                                                value: *duration_ns,
                                                                elapsed_nanos: elapsed.as_nanos() as u64,
                                                                alloc_count: None,
                                                                tid: *tid,
                                                                result: result_log.clone(),
                                                            })
                                                            .collect();
                                                    }
                                                }
                                                FunctionLogsList {
                                                    function_name: stats.name.to_string(),
                                                    logs,
                                                    count: stats.count as usize,
                                                }
                                            });
                                        let _ = response_tx.send(response);
                                    }
                                    FunctionsQuery::LogsAlloc { function_id, response_tx } => {
                                        cfg_if::cfg_if! {
                                            if #[cfg(feature = "hotpath-alloc")] {
                                                let response = local_stats.get(&function_id)
                                                    .map(|stats| {
                                                        let logs: Vec<FunctionLog> = stats.recent_logs
                                                            .iter()
                                                            .rev()
                                                            .map(|(bytes, count, _duration_ns, elapsed, tid, result_log)| FunctionLog {
                                                                value: *bytes,
                                                                elapsed_nanos: elapsed.as_nanos() as u64,
                                                                alloc_count: *count,
                                                                tid: *tid,
                                                                result: result_log.clone(),
                                                            })
                                                            .collect();
                                                        FunctionLogsList {
                                                            function_name: stats.name.to_string(),
                                                            logs,
                                                            count: stats.count as usize,
                                                        }
                                                    });
                                                let _ = response_tx.send(response);
                                            } else {
                                                let _ = function_id;
                                                let _ = response_tx.send(None);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                }

                let _ = completion_tx.send(local_stats);
            })
            .expect("Failed to spawn hotpath-worker thread");

        arc_swap.store(Some(Arc::clone(&state_arc)));
        set_measurements_active(true);

        crate::lib_on::START_TIME.get_or_init(Instant::now);

        crate::metrics_server::start_metrics_server_once(*METRICS_SERVER_PORT);

        #[cfg(feature = "hotpath-prometheus")]
        crate::prometheus_server::start_prometheus_server_once();

        #[cfg(feature = "hotpath-mcp")]
        crate::mcp_server::start_mcp_server_once();

        // In auto mode the future!/future_fn macros init state lazily on first
        // use; eager init is only needed when the section is requested by name.
        if sections_mode.explicitly_contains(Section::Futures) {
            crate::futures::init_futures_state();
        }

        #[cfg(feature = "threads")]
        {
            crate::threads::init_threads_monitoring();
        }

        let wrapper_guard = crate::functions::build_measurement_guard_sync(caller_name, true);

        drop(_suspend);

        #[cfg(all(feature = "threads", feature = "hotpath-alloc"))]
        crate::functions::alloc::core::init_thread_alloc_tracking();

        #[cfg(feature = "hotpath-cpu")]
        if sections_mode.contains_or_auto(Section::FunctionsCpu) {
            crate::functions::cpu::autospawn::start();
        }

        // Last, so guard setup is not counted.
        #[cfg(feature = "hotpath-callgrind")]
        crate::functions::callgrind::start_session();

        // Meta-internal sections (measurement transport, queue registration)
        // allocate while user-level tracking may be live; these hooks let the
        // meta profiler suspend our alloc tracking around them so those bytes
        // are not attributed to user functions.
        #[cfg(all(feature = "hotpath-meta", feature = "hotpath-alloc"))]
        hotpath_meta::set_host_alloc_suspend_hooks(
            crate::functions::alloc::core::suspend_alloc_tracking,
            crate::functions::alloc::core::resume_alloc_tracking,
        );

        #[cfg(feature = "hotpath-meta")]
        let _meta_guard = {
            let builder = hotpath_meta::HotpathGuardBuilder::new("hotpath-meta")
                .percentiles(&[95.0, 99.0, 99.9])
                .functions_limit(10)
                .threads_limit(5);
            if std::env::var("HOTPATH_META_SHUTDOWN_MS").is_ok() {
                builder.build_with_shutdown(std::time::Duration::from_secs(0));
                None
            } else {
                Some(builder.build())
            }
        };

        Self {
            state: Arc::clone(&state_arc),
            format,
            wrapper_guard: Some(wrapper_guard),
            output_path,
            sections_mode,
            start_time,
            before_shutdown,
            channels_limit,
            streams_limit,
            futures_limit,
            rw_locks_limit,
            mutexes_limit,
            sql_limit,
            http_limit,
            server_limit,
            io_limit,
            threads_limit,
            user_metadata,
            #[cfg(feature = "hotpath-meta")]
            _meta_guard,
        }
    }
}

fn parse_usize_env(name: &str) -> Option<usize> {
    std::env::var(name).ok().and_then(|s| s.parse().ok())
}

fn make_functions_config(
    state_guard: &FunctionsState,
    total_elapsed: std::time::Duration,
    limit_for: impl Fn(usize) -> usize,
    cloud: bool,
) -> FunctionStatsConfig {
    let display_limit = parse_usize_env("HOTPATH_FUNCTIONS_LIMIT")
        .or_else(|| parse_usize_env("HOTPATH_LIMIT"))
        .unwrap_or(state_guard.limit);

    FunctionStatsConfig {
        total_elapsed,
        percentiles: state_guard.percentiles.clone(),
        caller_name: state_guard.caller_name,
        limit: limit_for(display_limit),
        cloud,
    }
}

fn build_timing_list(
    stats: &HashMap<u32, FunctionStats>,
    config: &FunctionStatsConfig,
    elapsed_ns: u64,
) -> JsonFunctionsList {
    cfg_if::cfg_if! {
        if #[cfg(feature = "hotpath-alloc")] {
            build_functions_list_timing(stats, config, elapsed_ns)
        } else {
            build_functions_list(stats, config, elapsed_ns)
        }
    }
}

#[cfg(feature = "hotpath-callgrind")]
fn build_icount_list(
    report: &crate::functions::callgrind::IcountReport,
    stats: &HashMap<u32, FunctionStats>,
    config: &FunctionStatsConfig,
    elapsed_ns: u64,
) -> JsonFunctionsList {
    let calls = stats
        .values()
        .filter(|s| !s.wrapper && s.has_data)
        .map(|s| (s.name, (s.id, s.count)))
        .collect();
    crate::functions::callgrind::build_icount_list(report, &calls, config, elapsed_ns)
}

impl Drop for HotpathGuard {
    fn drop(&mut self) {
        let _suspend = crate::lib_on::SuspendAllocTracking::new();

        #[cfg(feature = "hotpath-cpu")]
        let cpu_report: Option<Result<crate::functions::cpu::CpuReport, String>> = if self
            .sections_mode
            .contains_or_auto(Section::FunctionsCpu)
        {
            let caller_name = self
                .state
                .read()
                .map(|state| state.caller_name)
                .unwrap_or("unknown");
            match crate::functions::cpu::autospawn::stop() {
                Ok(profile_path) => {
                    match crate::functions::cpu::build_cpu_report_from_path(
                        caller_name,
                        &profile_path,
                    ) {
                        Some(r) => {
                            info!(
                                    "cpu report: caller={caller_name} total_samples={} attributed_samples={} stats_rows={}",
                                    r.total_samples,
                                    r.attributed_samples,
                                    r.stats.len()
                                );
                            Some(Ok(r))
                        }
                        None => {
                            let msg = format!(
                                "no data parsed from samply profile {}",
                                profile_path.display()
                            );
                            warn!("cpu report: {msg}");
                            Some(Err(msg))
                        }
                    }
                }
                Err(e) => {
                    warn!("cpu report: {e}");
                    Some(Err(e))
                }
            }
        } else {
            None
        };

        if let Some(f) = self.before_shutdown.take() {
            f();
        }

        // After the callback (its measured calls are counted), before any
        // report building.
        #[cfg(feature = "hotpath-callgrind")]
        let icount_report = crate::functions::callgrind::finish_session();
        #[cfg(feature = "hotpath-callgrind")]
        let counting_icount = icount_report.is_some();
        #[cfg(not(feature = "hotpath-callgrind"))]
        let counting_icount = false;

        let wrapper_guard = self.wrapper_guard.take().unwrap();
        drop(wrapper_guard);

        // Stop producers before signalling shutdown: everything published up to
        // this point is caught by the worker's final sweep.
        set_measurements_active(false);

        let state: Arc<crate::lib_on::MetaRwLock<FunctionsState>> = Arc::clone(&self.state);
        let elapsed = self.start_time.elapsed();
        let percentiles = state
            .read()
            .map(|s| s.percentiles.clone())
            .unwrap_or_default();

        let (shutdown_tx, completion_rx, end_time) = {
            let Ok(mut state_guard) = state.write() else {
                return;
            };

            let shutdown_tx = state_guard.shutdown_tx.take();
            let end_time = Instant::now();

            let completion_rx = state_guard.completion_rx.take();
            (shutdown_tx, completion_rx, end_time)
        };

        if let Some(tx) = shutdown_tx {
            let _ = tx.send(());
        }

        let functions_stats =
            completion_rx.and_then(|rx_mutex| rx_mutex.lock().ok().and_then(|rx| rx.recv().ok()));

        // Drain every subsystem regardless of the configured sections: each
        // shutdown_* is a no-op when the subsystem was never instrumented, and
        // in auto mode data presence decides which sections appear.
        let channels_data = report::shutdown_channels();
        let streams_data = report::shutdown_streams();
        let futures_data = report::shutdown_futures();
        let rw_locks_data = report::shutdown_rw_locks();
        let mutexes_data = report::shutdown_mutexes();
        let sql_data = report::shutdown_sql();
        let http_data = report::shutdown_http();
        let server_data = report::shutdown_server();
        let io_data = report::shutdown_io();

        let sections: Vec<Section> = match &self.sections_mode {
            SectionsMode::Explicit(list) => list.clone(),
            SectionsMode::Auto { include, exclude } => {
                // Durations under Valgrind are meaningless, so its run
                // reports instructions in their place.
                let mut base = vec![if counting_icount {
                    Section::FunctionsIcount
                } else {
                    Section::FunctionsTiming
                }];
                #[cfg(feature = "hotpath-alloc")]
                base.push(Section::FunctionsAlloc);
                #[cfg(feature = "hotpath-cpu")]
                base.push(Section::FunctionsCpu);
                base.push(Section::Threads);

                Section::all()
                    .into_iter()
                    .filter(|s| {
                        if exclude.contains(s) {
                            return false;
                        }
                        base.contains(s)
                            || include.contains(s)
                            || match s {
                                Section::Channels => !channels_data.is_empty(),
                                Section::Streams => !streams_data.is_empty(),
                                Section::Futures => !futures_data.is_empty(),
                                Section::RwLocks => !rw_locks_data.is_empty(),
                                Section::Mutexes => !mutexes_data.is_empty(),
                                Section::Sql => !sql_data.is_empty(),
                                Section::Http => !http_data.is_empty(),
                                Section::Server => !server_data.is_empty(),
                                Section::Io => !io_data.is_empty(),
                                Section::Debug => report::has_debug_entries(),
                                _ => false,
                            }
                    })
                    .collect()
            }
        };

        let output = OutputDestination::from_path(self.output_path.take());
        crate::output_on::set_use_colors(
            matches!(output, OutputDestination::Stdout)
                && std::env::var("NO_COLOR").is_err()
                && std::io::IsTerminal::is_terminal(&std::io::stdout()),
        );
        let format = if std::env::var("HOTPATH_OUTPUT_FORMAT").is_ok() {
            Format::from_env()
        } else {
            self.format
        };

        if let Some(global) = parse_usize_env("HOTPATH_LIMIT") {
            self.channels_limit = global;
            self.streams_limit = global;
            self.futures_limit = global;
            self.rw_locks_limit = global;
            self.mutexes_limit = global;
            self.sql_limit = global;
            self.http_limit = global;
            self.server_limit = global;
            self.io_limit = global;
            self.threads_limit = global;
        }
        if let Some(v) = parse_usize_env("HOTPATH_CHANNELS_LIMIT") {
            self.channels_limit = v;
        }
        if let Some(v) = parse_usize_env("HOTPATH_STREAMS_LIMIT") {
            self.streams_limit = v;
        }
        if let Some(v) = parse_usize_env("HOTPATH_FUTURES_LIMIT") {
            self.futures_limit = v;
        }
        if let Some(v) = parse_usize_env("HOTPATH_RW_LOCKS_LIMIT") {
            self.rw_locks_limit = v;
        }
        if let Some(v) = parse_usize_env("HOTPATH_MUTEXES_LIMIT") {
            self.mutexes_limit = v;
        }
        if let Some(v) = parse_usize_env("HOTPATH_SQL_LIMIT") {
            self.sql_limit = v;
        }
        if let Some(v) = parse_usize_env("HOTPATH_HTTP_LIMIT") {
            self.http_limit = v;
        }
        if let Some(v) = parse_usize_env("HOTPATH_SERVER_LIMIT") {
            self.server_limit = v;
        }
        if let Some(v) = parse_usize_env("HOTPATH_IO_LIMIT") {
            self.io_limit = v;
        }
        if let Some(v) = parse_usize_env("HOTPATH_THREADS_LIMIT") {
            self.threads_limit = v;
        }
        #[cfg(feature = "hotpath-cloud")]
        let upload_enabled = crate::lib_on::cloud::enabled();
        #[cfg(not(feature = "hotpath-cloud"))]
        let upload_enabled = false;

        let is_json = matches!(format, Format::Json | Format::JsonPretty);
        // With the cloud feature on, a JSON report is an upload payload
        // (complete lists and histograms), whether this process posts it or a
        // relay job does.
        #[cfg(feature = "hotpath-cloud")]
        let cloud_report = upload_enabled || is_json;
        #[cfg(not(feature = "hotpath-cloud"))]
        let cloud_report = false;

        // A JSON report is opened only once its bytes exist (below), so a
        // failed merge never truncates an existing file. An unusable table
        // output must not block the cloud upload.
        let mut writer: Box<dyn Write> = if is_json {
            Box::new(std::io::sink())
        } else {
            match output.writer() {
                Ok(w) => w,
                Err(e) => {
                    eprintln!("Failed to create output writer: {}", e);
                    if !upload_enabled {
                        return;
                    }
                    Box::new(std::io::sink())
                }
            }
        };

        // `HOTPATH_MERGE_REPORT`: a failed merge writes no JSON, uploads
        // nothing and exits 1 whatever the upload mode.
        let merge_path = crate::lib_on::merge::MERGE_REPORT.as_deref();
        let mut merge_error = None;
        if merge_path.is_some() && !is_json && !upload_enabled {
            merge_error = Some(crate::lib_on::merge::MergeError::NothingToMergeInto);
        }

        // A failed upload under strict mode fails the process, but only after
        // the local report below is written: the run's own results must not
        // be lost to a cloud-side problem.
        #[cfg(feature = "hotpath-cloud")]
        let mut upload_failed = false;
        if is_json || upload_enabled {
            let mut report = JsonReport {
                label: std::env::var("HOTPATH_REPORT_LABEL")
                    .ok()
                    .filter(|s| !s.is_empty()),
                user_metadata: (!self.user_metadata.is_empty()).then(|| self.user_metadata.clone()),
                time_sampling: crate::lib_on::sampling::active_rates(),
                ..JsonReport::new(crate::lib_on::report_meta::build_meta(if counting_icount {
                    crate::json::JsonRun::Callgrind
                } else {
                    crate::json::JsonRun::Native
                }))
            };

            // A cloud report lists every entry (`0` is unlimited): the server
            // diffs reports by name and truncates for display itself, while a
            // client-side top-N cannot be undone.
            let limit_for = |display: usize| if cloud_report { 0 } else { display };

            for section in &sections {
                match section {
                    Section::FunctionsTiming => {
                        if let Some(ref stats) = functions_stats {
                            if let Ok(state_guard) = state.read() {
                                let total_elapsed = end_time.duration_since(state_guard.start_time);
                                let elapsed_ns = total_elapsed.as_nanos() as u64;
                                let config = make_functions_config(
                                    &state_guard,
                                    total_elapsed,
                                    limit_for,
                                    cloud_report,
                                );
                                report.functions_timing =
                                    Some(build_timing_list(stats, &config, elapsed_ns));
                            }
                        }
                    }
                    Section::FunctionsAlloc => {
                        cfg_if::cfg_if! {
                            if #[cfg(feature = "hotpath-alloc")] {
                                if let Some(ref stats) = functions_stats {
                                    if let Ok(state_guard) = state.read() {
                                        let total_elapsed = end_time.duration_since(state_guard.start_time);
                                        let elapsed_ns = total_elapsed.as_nanos() as u64;
                                        let config = make_functions_config(&state_guard, total_elapsed, limit_for, cloud_report);
                                        report.functions_alloc = Some(
                                            build_functions_list_alloc(stats, &config, elapsed_ns),
                                        );
                                    }
                                }
                            }
                        }
                    }
                    Section::FunctionsCpu =>
                    {
                        #[cfg(feature = "hotpath-cpu")]
                        match cpu_report.as_ref() {
                            Some(Ok(cpu)) => {
                                if let Ok(state_guard) = state.read() {
                                    let total_elapsed =
                                        end_time.duration_since(state_guard.start_time);
                                    let elapsed_ns = total_elapsed.as_nanos() as u64;
                                    let config = make_functions_config(
                                        &state_guard,
                                        total_elapsed,
                                        limit_for,
                                        cloud_report,
                                    );
                                    let list = crate::functions::cpu::build_cpu_json(
                                        cpu,
                                        total_elapsed,
                                        elapsed_ns,
                                        config.limit,
                                    );
                                    report.functions_cpu =
                                        Some(crate::json::JsonFunctionsCpu::Ok(list));
                                }
                            }
                            Some(Err(msg)) => {
                                report.functions_cpu = Some(crate::json::JsonFunctionsCpu::Error {
                                    message: msg.clone(),
                                });
                            }
                            None => {}
                        }
                    }
                    Section::FunctionsIcount =>
                    {
                        #[cfg(feature = "hotpath-callgrind")]
                        match icount_report.as_ref() {
                            Some(Ok(icount)) => {
                                if let (Some(stats), Ok(state_guard)) =
                                    (functions_stats.as_ref(), state.read())
                                {
                                    let total_elapsed =
                                        end_time.duration_since(state_guard.start_time);
                                    let config = make_functions_config(
                                        &state_guard,
                                        total_elapsed,
                                        limit_for,
                                        cloud_report,
                                    );
                                    report.functions_icount = Some(build_icount_list(
                                        icount,
                                        stats,
                                        &config,
                                        total_elapsed.as_nanos() as u64,
                                    ));
                                }
                            }
                            Some(Err(msg)) => {
                                eprintln!("[hotpath] icount report unavailable: {msg}");
                            }
                            None => {}
                        }
                    }
                    Section::Channels => {
                        if !channels_data.is_empty() {
                            report.channels = Some(report::collect_channels_json(
                                &channels_data,
                                limit_for(self.channels_limit),
                                elapsed,
                                &percentiles,
                                cloud_report,
                            ));
                        }
                    }
                    Section::Streams => {
                        if !streams_data.is_empty() {
                            report.streams = Some(report::collect_streams_json(
                                &streams_data,
                                limit_for(self.streams_limit),
                                elapsed,
                            ));
                        }
                    }
                    Section::Futures => {
                        if !futures_data.is_empty() {
                            report.futures = Some(report::collect_futures_json(
                                &futures_data,
                                limit_for(self.futures_limit),
                                elapsed,
                            ));
                        }
                    }
                    Section::RwLocks => {
                        if !rw_locks_data.is_empty() {
                            report.rw_locks = Some(report::collect_rw_locks_json(
                                &rw_locks_data,
                                limit_for(self.rw_locks_limit),
                                elapsed,
                                &percentiles,
                                cloud_report,
                            ));
                        }
                    }
                    Section::Mutexes => {
                        if !mutexes_data.is_empty() {
                            report.mutexes = Some(report::collect_mutexes_json(
                                &mutexes_data,
                                limit_for(self.mutexes_limit),
                                elapsed,
                                &percentiles,
                                cloud_report,
                            ));
                        }
                    }
                    Section::Sql => {
                        if !sql_data.is_empty() {
                            report.sql = Some(report::collect_sql_json(
                                &sql_data,
                                limit_for(self.sql_limit),
                                elapsed,
                                &percentiles,
                                cloud_report,
                            ));
                        }
                    }
                    Section::Http => {
                        if !http_data.is_empty() {
                            report.http = Some(report::collect_http_json(
                                &http_data,
                                limit_for(self.http_limit),
                                elapsed,
                                &percentiles,
                                cloud_report,
                            ));
                        }
                    }
                    Section::Server => {
                        if !server_data.is_empty() {
                            report.server = Some(report::collect_server_json(
                                &server_data,
                                limit_for(self.server_limit),
                                elapsed,
                                &percentiles,
                                report::ServerColumns::from_state(),
                                cloud_report,
                            ));
                        }
                    }
                    Section::Io => {
                        if !io_data.is_empty() {
                            report.io = Some(report::collect_io_json(
                                &io_data,
                                limit_for(self.io_limit),
                                elapsed,
                                &percentiles,
                                cloud_report,
                            ));
                        }
                    }
                    Section::Threads => {
                        #[cfg(feature = "threads")]
                        {
                            let json = report::collect_threads_json(
                                limit_for(self.threads_limit),
                                cloud_report,
                            );
                            if !json.data.is_empty() {
                                report.threads = Some(json);
                            }
                        }
                    }
                    Section::Debug => {
                        let json = report::collect_debug_json(elapsed, limit_for(0));
                        if !json.entries.is_empty() {
                            report.debug = Some(json);
                        }
                    }
                }
            }

            let report = match merge_path {
                None => Some(report),
                // The workload stopped early: its counts must not be merged
                // into a complete native report. The panic fails the process.
                Some(_) if std::thread::panicking() => {
                    eprintln!("[hotpath] merge skipped: the profiled program panicked");
                    None
                }
                Some(path) => {
                    #[cfg(feature = "hotpath-callgrind")]
                    let icount_run = match icount_report.as_ref() {
                        None => crate::lib_on::merge::IcountRun::NotUnderValgrind,
                        Some(Err(msg)) => crate::lib_on::merge::IcountRun::DumpFailed(msg),
                        Some(Ok(_)) => crate::lib_on::merge::IcountRun::Counted,
                    };
                    #[cfg(not(feature = "hotpath-callgrind"))]
                    let icount_run = crate::lib_on::merge::IcountRun::NotUnderValgrind;
                    let output_file = match &output {
                        OutputDestination::File(file) if is_json => Some(file.as_path()),
                        _ => None,
                    };
                    let merged = crate::lib_on::merge::apply(report, icount_run, path, output_file)
                        .and_then(|merged| {
                            // Compact, as uploaded and as a relay posts it.
                            let compact = serde_json::to_vec(&merged).map_err(|e| {
                                crate::lib_on::merge::MergeError::Serialize(e.to_string())
                            })?;
                            crate::json::cloud_api::check_report_size(compact.len())
                                .map(|()| merged)
                                .map_err(crate::lib_on::merge::MergeError::TooLarge)
                        });
                    match merged {
                        Ok(merged) => Some(merged),
                        Err(e) => {
                            merge_error = Some(e);
                            None
                        }
                    }
                }
            };

            if let Some(report) = report {
                let json = match format {
                    Format::Json => serde_json::to_vec(&report).ok(),
                    Format::JsonPretty => serde_json::to_vec_pretty(&report).ok(),
                    _ => None,
                };
                if let Some(mut json) = json {
                    json.push(b'\n');
                    // A merged report is replaced atomically; a plain one is
                    // written as before, so `/dev/stdout`-like paths keep
                    // working unchanged.
                    let written = if merge_path.is_some() {
                        output.write_atomically(&json)
                    } else {
                        output.writer().and_then(|mut w| w.write_all(&json))
                    };
                    if let Err(e) = written {
                        eprintln!("Failed to write the JSON report: {e}");
                    }
                }

                #[cfg(feature = "hotpath-cloud")]
                if upload_enabled {
                    upload_failed = crate::lib_on::cloud::upload(&report);
                }
            }
        }

        if !is_json {
            let label = std::env::var("HOTPATH_REPORT_LABEL")
                .ok()
                .filter(|s| !s.is_empty());
            if matches!(format, Format::Table) {
                write_report_header(&mut writer, elapsed, &sections, label.as_deref());
                if let Some(err) = crate::metrics_server::get_metrics_server_error() {
                    let _ = writeln!(writer, "[hotpath - error] {}", err);
                }
                write_user_metadata_table(&mut writer, &self.user_metadata);
            }

            for section in &sections {
                match section {
                    Section::FunctionsTiming => {
                        if let Some(ref stats) = functions_stats {
                            if let Ok(state_guard) = state.read() {
                                let total_elapsed = end_time.duration_since(state_guard.start_time);
                                let config = make_functions_config(
                                    &state_guard,
                                    total_elapsed,
                                    |limit| limit,
                                    false,
                                );
                                let elapsed_ns = total_elapsed.as_nanos() as u64;
                                let list = build_timing_list(stats, &config, elapsed_ns);

                                match format {
                                    Format::Table => {
                                        if list.data.is_empty() {
                                            display_no_measurements_message_to(
                                                &mut writer,
                                                total_elapsed,
                                                state_guard.caller_name,
                                            );
                                        } else {
                                            display_functions_table_to(&mut writer, &list);
                                        }
                                    }
                                    Format::None => {}
                                    _ => {}
                                }
                            }
                        }
                    }
                    Section::FunctionsAlloc => {
                        cfg_if::cfg_if! {
                            if #[cfg(feature = "hotpath-alloc")] {
                                if let Some(ref stats) = functions_stats {
                                    if let Ok(state_guard) = state.read() {
                                        let total_elapsed = end_time.duration_since(state_guard.start_time);
                                        let config = make_functions_config(&state_guard, total_elapsed, |limit| limit, false);
                                        let elapsed_ns = total_elapsed.as_nanos() as u64;
                                        let list = build_functions_list_alloc(stats, &config, elapsed_ns);

                                        match format {
                                            Format::Table => {
                                                if list.data.is_empty() {
                                                    display_no_measurements_message_to(
                                                        &mut writer,
                                                        total_elapsed,
                                                        state_guard.caller_name,
                                                    );
                                                } else {
                                                    display_functions_table_to(&mut writer, &list);
                                                }
                                            }
                                            Format::None => {}
                                            _ => {}
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Section::FunctionsCpu =>
                    {
                        #[cfg(feature = "hotpath-cpu")]
                        if matches!(format, Format::Table) {
                            match cpu_report.as_ref() {
                                Some(Ok(cpu)) => {
                                    if let Ok(state_guard) = state.read() {
                                        let total_elapsed =
                                            end_time.duration_since(state_guard.start_time);
                                        let elapsed_ns = total_elapsed.as_nanos() as u64;
                                        let config = make_functions_config(
                                            &state_guard,
                                            total_elapsed,
                                            |limit| limit,
                                            false,
                                        );
                                        let list = crate::functions::cpu::build_cpu_json(
                                            cpu,
                                            total_elapsed,
                                            elapsed_ns,
                                            config.limit,
                                        );
                                        crate::functions::cpu::report_functions_cpu_table(
                                            &mut writer,
                                            &list,
                                        );
                                    }
                                }
                                Some(Err(msg)) => {
                                    crate::functions::cpu::report_functions_cpu_error_table(
                                        &mut writer,
                                        msg,
                                    );
                                }
                                None => {}
                            }
                        }
                    }
                    Section::FunctionsIcount =>
                    {
                        #[cfg(feature = "hotpath-callgrind")]
                        if matches!(format, Format::Table) {
                            match icount_report.as_ref() {
                                Some(Ok(icount)) => {
                                    if let (Some(stats), Ok(state_guard)) =
                                        (functions_stats.as_ref(), state.read())
                                    {
                                        let total_elapsed =
                                            end_time.duration_since(state_guard.start_time);
                                        let config = make_functions_config(
                                            &state_guard,
                                            total_elapsed,
                                            |limit| limit,
                                            false,
                                        );
                                        let list = build_icount_list(
                                            icount,
                                            stats,
                                            &config,
                                            total_elapsed.as_nanos() as u64,
                                        );
                                        if list.data.is_empty() {
                                            display_no_measurements_message_to(
                                                &mut writer,
                                                total_elapsed,
                                                state_guard.caller_name,
                                            );
                                        } else {
                                            display_functions_table_to(&mut writer, &list);
                                        }
                                    }
                                }
                                Some(Err(msg)) => {
                                    let _ =
                                        writeln!(writer, "icount - report unavailable: {msg}\n");
                                }
                                None => {
                                    let _ = writeln!(
                                        writer,
                                        "icount - not running under valgrind --tool=callgrind\n"
                                    );
                                }
                            }
                        }
                    }
                    Section::Channels => {
                        if matches!(format, Format::Table) {
                            let total = channels_data.len();
                            let limit = apply_limit(total, self.channels_limit);
                            report::report_channels_table(
                                &channels_data[..limit],
                                total,
                                elapsed,
                                &mut writer,
                            );
                            report::report_channel_latency_table(
                                &channels_data[..limit],
                                &percentiles,
                                &mut writer,
                            );
                        }
                    }
                    Section::Streams => {
                        if matches!(format, Format::Table) {
                            let total = streams_data.len();
                            let limit = apply_limit(total, self.streams_limit);
                            report::report_streams_table(
                                &streams_data[..limit],
                                total,
                                &mut writer,
                            );
                        }
                    }
                    Section::Futures => {
                        if matches!(format, Format::Table) {
                            let total = futures_data.len();
                            let limit = apply_limit(total, self.futures_limit);
                            report::report_futures_table(
                                &futures_data[..limit],
                                total,
                                &mut writer,
                            );
                        }
                    }
                    Section::RwLocks => {
                        if matches!(format, Format::Table) {
                            let total = rw_locks_data.len();
                            let limit = apply_limit(total, self.rw_locks_limit);
                            report::report_rw_locks_table(
                                &rw_locks_data[..limit],
                                total,
                                &percentiles,
                                &mut writer,
                            );
                        }
                    }
                    Section::Mutexes => {
                        if matches!(format, Format::Table) {
                            let total = mutexes_data.len();
                            let limit = apply_limit(total, self.mutexes_limit);
                            report::report_mutexes_table(
                                &mutexes_data[..limit],
                                total,
                                &percentiles,
                                &mut writer,
                            );
                        }
                    }
                    Section::Sql => {
                        if matches!(format, Format::Table) {
                            let total = sql_data.len();
                            let reference_total: u64 = sql_data.iter().map(|e| e.total_nanos).sum();
                            let total_calls: u64 = sql_data.iter().map(|e| e.count).sum();
                            let limit = apply_limit(total, self.sql_limit);
                            report::report_sql_table(
                                &sql_data[..limit],
                                total,
                                total_calls,
                                reference_total,
                                &percentiles,
                                &mut writer,
                            );
                        }
                    }
                    Section::Http => {
                        if matches!(format, Format::Table) {
                            let total = http_data.len();
                            let reference_total: u64 =
                                http_data.iter().map(|e| e.total_nanos).sum();
                            let total_calls: u64 = http_data.iter().map(|e| e.count).sum();
                            let limit = apply_limit(total, self.http_limit);
                            report::report_http_table(
                                &http_data[..limit],
                                total,
                                total_calls,
                                reference_total,
                                &percentiles,
                                &mut writer,
                            );
                        }
                    }
                    Section::Server => {
                        if matches!(format, Format::Table) {
                            let total = server_data.len();
                            let reference_total: u64 =
                                server_data.iter().map(|e| e.total_nanos).sum();
                            let reference_alloc_bytes: u64 =
                                server_data.iter().map(|e| e.alloc_bytes).sum();
                            let total_calls: u64 = server_data.iter().map(|e| e.count).sum();
                            let limit = apply_limit(total, self.server_limit);
                            report::report_server_table(
                                &server_data[..limit],
                                total,
                                total_calls,
                                reference_total,
                                reference_alloc_bytes,
                                &percentiles,
                                report::ServerColumns::from_state(),
                                &mut writer,
                            );
                        }
                    }
                    Section::Io => {
                        if matches!(format, Format::Table) {
                            let total = io_data.len();
                            let limit = apply_limit(total, self.io_limit);
                            report::report_io_table(
                                &io_data[..limit],
                                total,
                                &percentiles,
                                &mut writer,
                            );
                        }
                    }
                    Section::Threads =>
                    {
                        #[cfg(feature = "threads")]
                        if matches!(format, Format::Table) {
                            report::report_threads_table(&mut writer, self.threads_limit);
                        }
                    }
                    Section::Debug => {
                        if matches!(format, Format::Table) {
                            report::report_debug_table(&mut writer);
                        }
                    }
                }
            }

            if matches!(format, Format::Table) {
                write_report_footer(&mut writer, &output, upload_enabled);
            }
        }

        if let Some(arc_swap) = FUNCTIONS_STATE.get() {
            arc_swap.store(None);
        }

        if let Some(e) = &merge_error {
            let message = format!("merge failed: {e}");
            #[cfg(feature = "hotpath-cloud")]
            crate::lib_on::cloud::emit_error(&message);
            #[cfg(not(feature = "hotpath-cloud"))]
            eprintln!("hotpath: {message}");
        }
        #[cfg(feature = "hotpath-cloud")]
        let must_exit = upload_failed || merge_error.is_some();
        #[cfg(not(feature = "hotpath-cloud"))]
        let must_exit = merge_error.is_some();
        if must_exit {
            drop(writer);
            let _ = std::io::stdout().flush();
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::lib_on::hotpath_guard::{
        bounded_key_with_limit, normalize_percentiles, parse_user_metadata, MAX_PERCENTILES,
        OVERFLOW_ENTRY,
    };

    #[test]
    fn user_metadata_parses_pairs() {
        let parsed = parse_user_metadata(" env = ci , commit=abc=def,empty=").unwrap();
        let pairs: Vec<(&str, &str)> = parsed
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        assert_eq!(
            pairs,
            vec![("commit", "abc=def"), ("empty", ""), ("env", "ci")]
        );
        assert!(parse_user_metadata("").unwrap().is_empty());
        assert!(parse_user_metadata("   ").unwrap().is_empty());
    }

    #[test]
    fn user_metadata_rejects_malformed_input() {
        let missing = parse_user_metadata("env").unwrap_err();
        assert!(missing.contains("expected 'key=value'"), "{missing}");
        let trailing = parse_user_metadata("env=ci,").unwrap_err();
        assert!(trailing.contains("got ''"), "{trailing}");
        let empty_key = parse_user_metadata("=ci").unwrap_err();
        assert!(empty_key.contains("empty key"), "{empty_key}");
        let duplicate = parse_user_metadata("env=ci,env=local").unwrap_err();
        assert!(duplicate.contains("duplicate key 'env'"), "{duplicate}");
    }

    #[test]
    fn bounded_key_folds_new_keys_into_overflow_once_full() {
        let mut map: HashMap<String, u32> = HashMap::new();
        let overflow = || OVERFLOW_ENTRY.to_string();

        // Limit 3 leaves room for two regular keys plus the overflow slot.
        for key in ["a", "b"] {
            let k = bounded_key_with_limit(&map, 3, key.to_string(), overflow);
            assert_eq!(k, key);
            map.insert(k, 1);
        }

        // Regular slots full: unknown key goes to the overflow bucket, known keys pass.
        assert_eq!(
            bounded_key_with_limit(&map, 3, "c".to_string(), overflow),
            OVERFLOW_ENTRY
        );
        assert_eq!(
            bounded_key_with_limit(&map, 3, "a".to_string(), overflow),
            "a"
        );

        // With the overflow bucket present the map sits exactly at the limit.
        map.insert(OVERFLOW_ENTRY.to_string(), 1);
        assert_eq!(map.len(), 3);
        assert_eq!(
            bounded_key_with_limit(&map, 3, "d".to_string(), overflow),
            OVERFLOW_ENTRY
        );
    }

    #[test]
    fn normalize_percentiles_sorts_and_deduplicates() {
        assert_eq!(
            normalize_percentiles(&[99.0, 50.0, 95.0, 50.0]),
            vec![50.0, 95.0, 99.0]
        );
        assert_eq!(normalize_percentiles(&[]), Vec::<f64>::new());
    }

    #[test]
    fn normalize_percentiles_counts_distinct_values_against_the_limit() {
        let at_limit: Vec<f64> = (0..MAX_PERCENTILES).map(|i| i as f64).collect();
        assert_eq!(normalize_percentiles(&at_limit).len(), MAX_PERCENTILES);

        // Duplicates collapse before the limit is applied.
        let repeated = vec![95.0; MAX_PERCENTILES + 5];
        assert_eq!(normalize_percentiles(&repeated), vec![95.0]);
    }

    #[test]
    #[should_panic(expected = "Too many percentiles")]
    fn normalize_percentiles_rejects_too_many() {
        let values: Vec<f64> = (0..=MAX_PERCENTILES).map(|i| i as f64).collect();
        normalize_percentiles(&values);
    }

    #[test]
    #[should_panic(expected = "must be 0..=100")]
    fn normalize_percentiles_rejects_out_of_range() {
        normalize_percentiles(&[50.0, 101.0]);
    }

    #[test]
    #[should_panic(expected = "must be 0..=100")]
    fn normalize_percentiles_rejects_nan() {
        normalize_percentiles(&[f64::NAN]);
    }
}

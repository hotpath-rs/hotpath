//! Prometheus exporter (`hotpath-prometheus` feature), served from its own
//! tiny_http server (`HOTPATH_PROMETHEUS_HOST`/`HOTPATH_PROMETHEUS_PORT`,
//! auth via `HOTPATH_PROMETHEUS_AUTH_TOKEN`). One `GET /metrics` endpoint
//! with two bodies negotiated via the `Accept` header: protobuf with native
//! histograms (the high-resolution path) for scrapers that ask for it, text
//! exposition with coarse classic buckets for everything else. What
//! Prometheus ingests is decided by its own scrape config
//! (`scrape_native_histograms`), never here.

pub(crate) mod families;
pub(crate) mod protobuf;

use std::sync::{LazyLock, OnceLock};
use std::thread;

use tiny_http::{Header, Request, Response, Server};

use crate::prometheus_server::families::{
    to_protobuf, to_text, Family, FamilyKind, HistogramValue, Sample, SampleValue,
};

pub(crate) static PROMETHEUS_PORT: LazyLock<u16> = LazyLock::new(|| {
    std::env::var("HOTPATH_PROMETHEUS_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(6772)
});

/// Bind address for the exporter. Defaults to loopback; set to e.g. `0.0.0.0`
/// when a Prometheus container must reach the exporter through the Docker
/// bridge gateway (`host.docker.internal` on native Linux).
pub(crate) static PROMETHEUS_HOST: LazyLock<String> = LazyLock::new(|| {
    std::env::var("HOTPATH_PROMETHEUS_HOST").unwrap_or_else(|_| "127.0.0.1".to_string())
});

static PROMETHEUS_AUTH_TOKEN: LazyLock<Option<String>> =
    LazyLock::new(|| crate::auth::token_from_env("HOTPATH_PROMETHEUS_AUTH_TOKEN"));

// Per worker query; a scrape issues two, so the worst case stays under
// Prometheus' default 10s scrape timeout.
pub(crate) static RECV_TIMEOUT_MS: u64 = 4000;

static PROMETHEUS_SERVER_STARTED: OnceLock<()> = OnceLock::new();
static PROMETHEUS_SERVER_ERROR: OnceLock<String> = OnceLock::new();

/// Native histogram resolution: bucket ratio `2^(1/8)` ~ 9%. Locked after
/// benchmarking schemas 2/3/4: conversion cost is schema-independent, and 3
/// keeps worst-case sparse bucket counts within common
/// `native_histogram_bucket_limit` settings where 4 does not.
pub(crate) const NATIVE_SCHEMA: i32 = 3;

/// Bucket boundaries (ns) for the classic-format fallback - log-spaced 1-3
/// steps, coarse on purpose: native histograms are the high-resolution path.
/// `+Inf` is emitted by the renderer, never stored here.
const DEFAULT_FAST_LADDER_NS: &[u64] = &[
    250,            // 250ns
    1_000,          // 1µs
    3_000,          // 3µs
    10_000,         // 10µs
    30_000,         // 30µs
    100_000,        // 100µs
    300_000,        // 300µs
    1_000_000,      // 1ms
    3_000_000,      // 3ms
    10_000_000,     // 10ms
    30_000_000,     // 30ms
    100_000_000,    // 100ms
    300_000_000,    // 300ms
    1_000_000_000,  // 1s
    3_000_000_000,  // 3s
    10_000_000_000, // 10s
];

/// Bucket boundaries (ns) for I/O-bound latencies (SQL queries, HTTP client
/// requests, server responses) - same log-spaced 1-3 stepping as
/// `DEFAULT_FAST_LADDER_NS`, shifted up: sub-100µs I/O is indistinguishably
/// "fast" and a p99 above the top boundary would clip to it and silently
/// understate.
const DEFAULT_SLOW_LADDER_NS: &[u64] = &[
    100_000,        // 100µs
    300_000,        // 300µs
    1_000_000,      // 1ms
    3_000_000,      // 3ms
    10_000_000,     // 10ms
    30_000_000,     // 30ms
    100_000_000,    // 100ms
    300_000_000,    // 300ms
    1_000_000_000,  // 1s
    3_000_000_000,  // 3s
    10_000_000_000, // 10s
    30_000_000_000, // 30s
    60_000_000_000, // 60s
];

/// Classic ladder for function, lock, channel, future and I/O durations:
/// `HOTPATH_PROMETHEUS_FAST_BUCKETS` (seconds, comma-separated) or
/// `DEFAULT_FAST_LADDER_NS`.
pub(crate) static FAST_LADDER_NS: LazyLock<Vec<u64>> =
    LazyLock::new(|| ladder_from_env("HOTPATH_PROMETHEUS_FAST_BUCKETS", DEFAULT_FAST_LADDER_NS));

/// Classic ladder for SQL, HTTP client and server durations:
/// `HOTPATH_PROMETHEUS_SLOW_BUCKETS` (seconds, comma-separated) or
/// `DEFAULT_SLOW_LADDER_NS`.
pub(crate) static SLOW_LADDER_NS: LazyLock<Vec<u64>> =
    LazyLock::new(|| ladder_from_env("HOTPATH_PROMETHEUS_SLOW_BUCKETS", DEFAULT_SLOW_LADDER_NS));

fn ladder_from_env(var: &str, default: &[u64]) -> Vec<u64> {
    match std::env::var(var) {
        Ok(raw) => parse_ladder(&raw).unwrap_or_else(|e| panic!("{var}: {e}")),
        Err(_) => default.to_vec(),
    }
}

/// Parses a comma-separated list of bucket boundaries in seconds into ns.
/// Every entry must be a finite positive number no larger than the duration
/// histograms' `MAX_DURATION_NS` ceiling (a higher bucket could never receive
/// a distinct count), and the list must be strictly ascending once rounded to
/// whole nanoseconds (Prometheus rejects duplicate `le` labels).
fn parse_ladder(raw: &str) -> Result<Vec<u64>, String> {
    let mut ladder = Vec::new();
    for item in raw.split(',') {
        let item = item.trim();
        let secs: f64 = item.parse().map_err(|_| {
            format!("invalid bucket boundary `{item}`, expected seconds like 0.001")
        })?;
        if !secs.is_finite() || secs <= 0.0 {
            return Err(format!(
                "bucket boundary `{item}` must be a positive number"
            ));
        }
        let ns = (secs * 1e9).round() as u64;
        if ns == 0 {
            return Err(format!("bucket boundary `{item}` is below 1ns"));
        }
        if ns > crate::lib_on::MAX_DURATION_NS {
            return Err(format!(
                "bucket boundary `{item}` exceeds the {}s maximum",
                crate::lib_on::MAX_DURATION_NS / 1_000_000_000
            ));
        }
        if ladder.last().is_some_and(|&prev| ns <= prev) {
            return Err(format!(
                "bucket boundaries must be strictly ascending, `{item}` does not exceed the previous entry"
            ));
        }
        ladder.push(ns);
    }
    Ok(ladder)
}

/// Bucket boundaries (bytes) for per-call allocation totals - powers of 4
/// from 64B to 1GiB, matching the alloc histograms' 1GB value clamp.
pub(crate) const ALLOC_LADDER_BYTES: &[u64] = &[
    64,
    256,
    1_024,
    4_096,
    16_384,
    65_536,
    262_144,
    1_048_576,
    4_194_304,
    16_777_216,
    67_108_864,
    268_435_456,
    1_073_741_824,
];

/// Bucket boundaries (allocation counts) for per-call allocation counts -
/// powers of 4 from 1 to ~1M.
pub(crate) const ALLOC_LADDER_COUNT: &[u64] = &[
    1, 4, 16, 64, 256, 1_024, 4_096, 16_384, 65_536, 262_144, 1_048_576,
];

const TEXT_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";
const PROTOBUF_CONTENT_TYPE: &str =
    "application/vnd.google.protobuf; proto=io.prometheus.client.MetricFamily; encoding=delimited";

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn start_prometheus_server_once() {
    PROMETHEUS_SERVER_STARTED.get_or_init(|| {
        start_prometheus_server(*PROMETHEUS_PORT);
    });
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
fn start_prometheus_server(port: u16) {
    LazyLock::force(&PROMETHEUS_AUTH_TOKEN);
    LazyLock::force(&FAST_LADDER_NS);
    LazyLock::force(&SLOW_LADDER_NS);
    crate::dev_logging::init_logging();

    thread::Builder::new()
        .name("hp-prometheus".into())
        .spawn(move || {
            let _suspend = crate::lib_on::SuspendAllocTracking::new();
            let addr = format!("{}:{}", *PROMETHEUS_HOST, port);
            crate::dev_logging::info!("prometheus: exporter listening on {}", addr);
            let server = match Server::http(&addr) {
                Ok(s) => s,
                Err(e) => {
                    let msg = format!(
                        "{} busy ({}), skipping Prometheus server start. Use HOTPATH_PROMETHEUS_PORT to change the port.",
                        addr, e
                    );
                    eprintln!("[hotpath - error] {}", msg);
                    let _ = PROMETHEUS_SERVER_ERROR.set(msg);
                    return;
                }
            };

            for request in server.incoming_requests() {
                handle_request(request);
            }
        })
        .expect("Failed to spawn Prometheus server thread");
}

/// Prometheus's `authorization` scrape config sends `Bearer <token>`, so
/// accept the bare token or a `Bearer `-prefixed one.
fn check_auth_with_bearer(expected: Option<&str>, provided: Option<&str>) -> bool {
    crate::auth::check_auth(expected, provided)
        || crate::auth::check_auth(expected, provided.and_then(|p| p.strip_prefix("Bearer ")))
}

fn accepts_protobuf(accept: &str) -> bool {
    accept.contains("application/vnd.google.protobuf") && accept.contains("delimited")
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
fn handle_request(request: Request) {
    let provided = request
        .headers()
        .iter()
        .find(|h| h.field.equiv("Authorization"))
        .map(|h| h.value.as_str());
    if !check_auth_with_bearer(PROMETHEUS_AUTH_TOKEN.as_deref(), provided) {
        crate::dev_logging::warn!("prometheus: unauthorized request to {}", request.url());
        respond_text(request, 401, "Unauthorized\n");
        return;
    }

    let path = request.url().split('?').next().unwrap_or("");
    if path != "/metrics" {
        crate::dev_logging::warn!("prometheus: unknown path {}", path);
        respond_text(request, 404, "Not found\n");
        return;
    }

    let accept = request
        .headers()
        .iter()
        .find(|h| h.field.equiv("Accept"))
        .map(|h| h.value.as_str().to_string())
        .unwrap_or_default();
    let user_agent = request
        .headers()
        .iter()
        .find(|h| h.field.equiv("User-Agent"))
        .map(|h| h.value.as_str().to_string())
        .unwrap_or_default();

    let protobuf = accepts_protobuf(&accept);
    let body = collect_families().and_then(|families| {
        if protobuf {
            to_protobuf(&families).map(|body| (body, PROTOBUF_CONTENT_TYPE))
        } else {
            Some((to_text(&families).into_bytes(), TEXT_CONTENT_TYPE))
        }
    });

    match body {
        Some((body, content_type)) => {
            crate::dev_logging::info!(
                "prometheus: scrape format={} bytes={} user_agent={:?} accept={:?}",
                if protobuf { "protobuf" } else { "text" },
                body.len(),
                user_agent,
                accept
            );
            respond_bytes(request, body, content_type);
        }
        // An empty scrape would fabricate counter resets in Prometheus, so a
        // timed-out or not-yet-started worker yields an error status instead.
        None => {
            crate::dev_logging::warn!("prometheus: worker not ready, responding 503");
            respond_text(request, 503, "Profiler worker not ready\n");
        }
    }
}

/// Builds every metric family for the current scrape. `None` when the
/// functions snapshot cannot be fetched (worker not started or timed out) -
/// the caller responds 503 instead of serving an empty scrape.
#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
fn collect_families() -> Option<Vec<Family>> {
    let functions = crate::functions::get_functions_raw()?;

    let mut families = vec![
        Family {
            name: "hotpath_build_info",
            help: "hotpath build info, value is always 1.",
            kind: FamilyKind::Gauge,
            samples: vec![Sample {
                labels: vec![
                    ("hotpath_version", env!("CARGO_PKG_VERSION").to_string()),
                    ("rustc_version", env!("HOTPATH_RUSTC_VERSION").to_string()),
                    ("profile", env!("HOTPATH_CARGO_PROFILE").to_string()),
                    (
                        "os",
                        format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
                    ),
                ],
                value: SampleValue::Scalar(1.0),
            }],
        },
        Family {
            name: "hotpath_uptime_seconds",
            help: "Seconds since profiling started.",
            kind: FamilyKind::Gauge,
            samples: vec![Sample {
                labels: vec![],
                value: SampleValue::Scalar(seconds(crate::lib_on::current_elapsed_ns())),
            }],
        },
    ];

    if !functions.is_empty() {
        families.push(Family {
            name: "hotpath_function_calls_total",
            help: "Total calls of each instrumented function, including calls skipped by time sampling.",
            kind: FamilyKind::Counter,
            samples: functions
                .iter()
                .map(|f| Sample {
                    labels: vec![("function", f.name.to_string())],
                    value: SampleValue::Scalar(f.count as f64),
                })
                .collect(),
        });

        collect_functions_routes(&mut families, &functions);

        families.push(Family {
            name: "hotpath_function_duration_seconds",
            help: "Duration of sampled calls of each instrumented function.",
            kind: FamilyKind::Histogram,
            samples: functions
                .into_iter()
                .map(|f| Sample {
                    labels: vec![("function", f.name.to_string())],
                    value: SampleValue::Histogram(HistogramValue {
                        sample_count: f.sampled_count,
                        sum: seconds(f.total_duration_ns),
                        classic_buckets: classic_pairs(&FAST_LADDER_NS, f.bucket_counts),
                        native_buckets: f.native_buckets,
                        zero_count: 0,
                    }),
                })
                .collect(),
        });
    }

    collect_functions_alloc(&mut families)?;
    collect_sql(&mut families);
    collect_http(&mut families);
    collect_server(&mut families);
    collect_mutexes(&mut families);
    collect_rw_locks(&mut families);
    collect_channels(&mut families);
    collect_streams(&mut families);
    collect_io(&mut families);
    collect_futures(&mut families);
    #[cfg(feature = "threads")]
    collect_threads(&mut families);
    #[cfg(feature = "tokio")]
    collect_tokio_runtime(&mut families);
    collect_gauges(&mut families);

    Some(families)
}

/// One `(function, route)` sample per route a function ran under, from the
/// per-route slices every function keeps (see `RouteFunctionStats`).
fn route_samples<'a>(
    functions: impl IntoIterator<Item = (&'a str, &'a [crate::functions::RawRouteFunction])>,
    value: impl Fn(&crate::functions::RouteFunctionStats) -> f64,
) -> Vec<Sample> {
    let value = &value;
    functions
        .into_iter()
        .flat_map(|(name, routes)| {
            routes.iter().map(move |r| Sample {
                labels: vec![
                    ("function", name.to_string()),
                    ("route", r.route.to_string()),
                ],
                value: SampleValue::Scalar(value(&r.stats)),
            })
        })
        .collect()
}

/// Calls and time of each function split by the axum route they ran under;
/// running totals only, the per-function families keep the distributions.
#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
fn collect_functions_routes(
    families: &mut Vec<Family>,
    functions: &[crate::functions::RawFunctionTiming],
) {
    if functions.iter().all(|f| f.routes.is_empty()) {
        return;
    }
    let slices = || functions.iter().map(|f| (f.name, f.routes.as_slice()));

    families.push(Family {
        name: "hotpath_function_route_calls_total",
        help: "Calls of each instrumented function made under an axum route scope, per route.",
        kind: FamilyKind::Counter,
        samples: route_samples(slices(), |s| s.count as f64),
    });
    families.push(Family {
        name: "hotpath_function_route_timed_calls_total",
        help: "Route-scoped calls that carried a duration (not skipped by time sampling); the denominator for per-call time.",
        kind: FamilyKind::Counter,
        samples: route_samples(slices(), |s| s.sampled_count as f64),
    });
    families.push(Family {
        name: "hotpath_function_route_duration_seconds_total",
        help: "Time spent in each instrumented function under each axum route, over timed calls.",
        kind: FamilyKind::Counter,
        samples: route_samples(slices(), |s| seconds(s.total_duration_ns)),
    });
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
/// `None` aborts the scrape (worker unreachable or timed out - serving 200
/// with the alloc series missing would make Prometheus mark them stale and
/// fabricate resets); a reachable worker without hotpath-alloc just omits the
/// families.
fn collect_functions_alloc(families: &mut Vec<Family>) -> Option<()> {
    let alloc = crate::functions::get_functions_alloc_raw()?;
    let Some(functions) = alloc else {
        return Some(());
    };
    if functions.is_empty() {
        return Some(());
    }

    let labels = |f: &crate::functions::RawFunctionAlloc| vec![("function", f.name.to_string())];

    families.push(Family {
        name: "hotpath_function_alloc_bytes_total",
        help: "Total bytes allocated by each instrumented function.",
        kind: FamilyKind::Counter,
        samples: functions
            .iter()
            .map(|f| Sample {
                labels: labels(f),
                value: SampleValue::Scalar(f.total_bytes as f64),
            })
            .collect(),
    });

    families.push(Family {
        name: "hotpath_function_alloc_count_total",
        help: "Total allocations made by each instrumented function.",
        kind: FamilyKind::Counter,
        samples: functions
            .iter()
            .map(|f| Sample {
                labels: labels(f),
                value: SampleValue::Scalar(f.total_allocs as f64),
            })
            .collect(),
    });

    // Async entries whose measurements carried no per-call totals export the
    // counters above only; their histogram projections are empty.
    let bytes_samples: Vec<Sample> = functions
        .iter()
        .filter(|f| f.bytes_sample_count > 0)
        .map(|f| Sample {
            labels: labels(f),
            value: SampleValue::Histogram(HistogramValue {
                sample_count: f.bytes_sample_count,
                sum: f.total_bytes as f64,
                classic_buckets: classic_pairs_units(ALLOC_LADDER_BYTES, f.bytes_classic.clone()),
                native_buckets: f.bytes_native.clone(),
                zero_count: f.bytes_zero_count,
            }),
        })
        .collect();
    if !bytes_samples.is_empty() {
        families.push(Family {
            name: "hotpath_function_alloc_bytes",
            help: "Bytes allocated per call of each instrumented function; values clamp at 1GB.",
            kind: FamilyKind::Histogram,
            samples: bytes_samples,
        });
    }

    if functions.iter().any(|f| !f.routes.is_empty()) {
        let slices = || functions.iter().map(|f| (f.name, f.routes.as_slice()));
        families.push(Family {
            name: "hotpath_function_route_alloc_bytes_total",
            help: "Bytes allocated by each instrumented function under each axum route.",
            kind: FamilyKind::Counter,
            samples: route_samples(slices(), |s| s.total_bytes as f64),
        });
        families.push(Family {
            name: "hotpath_function_route_alloc_count_total",
            help: "Allocations made by each instrumented function under each axum route.",
            kind: FamilyKind::Counter,
            samples: route_samples(slices(), |s| s.total_allocs as f64),
        });
    }

    let allocs_samples: Vec<Sample> = functions
        .iter()
        .filter(|f| f.allocs_sample_count > 0)
        .map(|f| Sample {
            labels: labels(f),
            value: SampleValue::Histogram(HistogramValue {
                sample_count: f.allocs_sample_count,
                sum: f.total_allocs as f64,
                classic_buckets: classic_pairs_units(ALLOC_LADDER_COUNT, f.allocs_classic.clone()),
                native_buckets: f.allocs_native.clone(),
                zero_count: f.allocs_zero_count,
            }),
        })
        .collect();
    if !allocs_samples.is_empty() {
        families.push(Family {
            name: "hotpath_function_alloc_count",
            help: "Allocations made per call of each instrumented function.",
            kind: FamilyKind::Histogram,
            samples: allocs_samples,
        });
    }

    Some(())
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
fn collect_io(families: &mut Vec<Family>) {
    crate::lib_on::io::with_sorted_io_entries(|entries| collect_io_entries(families, entries));
}

fn collect_io_entries(families: &mut Vec<Family>, entries: &[&crate::lib_on::io::IoEntry]) {
    use crate::lib_on::io::IoOpKind;

    let entries: Vec<&crate::lib_on::io::IoEntry> = entries
        .iter()
        .copied()
        .filter(|e| !e.key.is_empty())
        .collect();
    if entries.is_empty() {
        return;
    }

    const OPS: [(IoOpKind, &str); 4] = [
        (IoOpKind::Read, "read"),
        (IoOpKind::Write, "write"),
        (IoOpKind::Flush, "flush"),
        (IoOpKind::Shutdown, "shutdown"),
    ];

    let labels = |e: &crate::lib_on::io::IoEntry, op: &str| {
        let mut labels = call_site_labels(e.key, e.label.as_deref(), e.iter);
        labels.push(("type", e.type_name.to_string()));
        labels.push(("op", op.to_string()));
        labels
    };

    // Op kinds a wrapper never touched are skipped rather than exported as
    // all-zero series; errors count as activity (a retryable-failing reader
    // records errors without any completed op).
    let op_samples =
        |entries: &[&crate::lib_on::io::IoEntry],
         value: &dyn Fn(&crate::lib_on::io::IoOpStats) -> SampleValue| {
            entries
                .iter()
                .flat_map(|e| {
                    OPS.iter().filter_map(move |&(kind, op)| {
                        let stats = e.op(kind);
                        (stats.count > 0 || stats.errors > 0).then(|| Sample {
                            labels: labels(e, op),
                            value: value(stats),
                        })
                    })
                })
                .collect::<Vec<_>>()
        };

    families.push(Family {
        name: "hotpath_io_ops_total",
        help: "Total I/O operations per wrapper call site and op kind, including ops skipped by time sampling.",
        kind: FamilyKind::Counter,
        samples: op_samples(&entries, &|s| SampleValue::Scalar(s.count as f64)),
    });

    families.push(Family {
        name: "hotpath_io_bytes_total",
        help: "Total bytes transferred per wrapper call site and op kind.",
        kind: FamilyKind::Counter,
        samples: op_samples(&entries, &|s| SampleValue::Scalar(s.bytes as f64)),
    });

    families.push(Family {
        name: "hotpath_io_sampled_bytes_total",
        help: "Bytes transferred by timed operations; divide its rate by rate(hotpath_io_op_seconds_sum) for throughput that stays correct under sampling.",
        kind: FamilyKind::Counter,
        samples: op_samples(&entries, &|s| SampleValue::Scalar(s.sampled_bytes as f64)),
    });

    families.push(Family {
        name: "hotpath_io_errors_total",
        help: "I/O operations that returned an error, per wrapper call site and op kind.",
        kind: FamilyKind::Counter,
        samples: op_samples(&entries, &|s| SampleValue::Scalar(s.errors as f64)),
    });

    families.push(Family {
        name: "hotpath_io_op_seconds",
        help: "Duration of sampled I/O operations per wrapper call site and op kind.",
        kind: FamilyKind::Histogram,
        samples: op_samples(&entries, &|s| {
            SampleValue::Histogram(HistogramValue {
                sample_count: s.sampled_count,
                sum: seconds(s.total_nanos),
                classic_buckets: classic_pairs(&FAST_LADDER_NS, s.classic_buckets(&FAST_LADDER_NS)),
                native_buckets: s.native_buckets(NATIVE_SCHEMA),
                zero_count: 0,
            })
        }),
    });
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
fn collect_futures(families: &mut Vec<Family>) {
    let entries = crate::lib_on::futures::get_sorted_future_stats();
    if entries.is_empty() {
        return;
    }

    let labels =
        |e: &crate::lib_on::futures::FutureEntry| future_labels(e.source, e.label.as_deref());

    families.push(Family {
        name: "hotpath_future_polls_total",
        help: "Total polls of each instrumented future, including polls skipped by time sampling.",
        kind: FamilyKind::Counter,
        samples: entries
            .iter()
            .map(|e| Sample {
                labels: labels(e),
                value: SampleValue::Scalar(e.total_poll_count as f64),
            })
            .collect(),
    });

    families.push(Family {
        name: "hotpath_future_sampled_polls_total",
        help: "Timed polls; the denominator for the average poll duration: rate(hotpath_future_poll_seconds_total) / rate(hotpath_future_sampled_polls_total).",
        kind: FamilyKind::Counter,
        samples: entries
            .iter()
            .map(|e| Sample {
                labels: labels(e),
                value: SampleValue::Scalar(e.sampled_polls as f64),
            })
            .collect(),
    });

    families.push(Family {
        name: "hotpath_future_poll_seconds_total",
        help: "Time spent in timed polls of each instrumented future.",
        kind: FamilyKind::Counter,
        samples: entries
            .iter()
            .map(|e| Sample {
                labels: labels(e),
                value: SampleValue::Scalar(seconds(e.total_poll_duration_ns)),
            })
            .collect(),
    });

    let alloc_bytes: Vec<Sample> = entries
        .iter()
        .filter_map(|e| {
            e.total_poll_alloc_bytes.map(|bytes| Sample {
                labels: labels(e),
                value: SampleValue::Scalar(bytes as f64),
            })
        })
        .collect();
    if !alloc_bytes.is_empty() {
        families.push(Family {
            name: "hotpath_future_poll_alloc_bytes_total",
            help: "Bytes allocated during polls of each instrumented future.",
            kind: FamilyKind::Counter,
            samples: alloc_bytes,
        });
    }

    let alloc_counts: Vec<Sample> = entries
        .iter()
        .filter_map(|e| {
            e.total_poll_alloc_count.map(|count| Sample {
                labels: labels(e),
                value: SampleValue::Scalar(count as f64),
            })
        })
        .collect();
    if !alloc_counts.is_empty() {
        families.push(Family {
            name: "hotpath_future_poll_allocs_total",
            help: "Allocations made during polls of each instrumented future.",
            kind: FamilyKind::Counter,
            samples: alloc_counts,
        });
    }
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
fn collect_mutexes(families: &mut Vec<Family>) {
    crate::lib_on::mutexes::with_sorted_mutex_entries(|entries| {
        collect_mutexes_entries(families, entries)
    });
}

fn collect_mutexes_entries(
    families: &mut Vec<Family>,
    entries: &[&crate::lib_on::mutexes::MutexEntry],
) {
    // An empty key marks a placeholder whose `Created` event has not been
    // processed yet: no identity labels, so exporting it would fabricate
    // series (and duplicates, given several placeholders). Its counts appear
    // once registration lands, at worst one sweep later.
    let entries: Vec<&crate::lib_on::mutexes::MutexEntry> = entries
        .iter()
        .copied()
        .filter(|e| !e.key.is_empty())
        .collect();
    if entries.is_empty() {
        return;
    }

    let labels = |e: &crate::lib_on::mutexes::MutexEntry| {
        call_site_labels(e.key, e.label.as_deref(), e.iter)
    };

    families.push(Family {
        name: "hotpath_mutex_acquisitions_total",
        help: "Total lock acquisitions per mutex call site, including acquisitions skipped by time sampling.",
        kind: FamilyKind::Counter,
        samples: entries
            .iter()
            .map(|e| Sample {
                labels: labels(e),
                value: SampleValue::Scalar(e.count as f64),
            })
            .collect(),
    });

    families.push(Family {
        name: "hotpath_mutex_wait_seconds",
        help: "Time sampled acquisitions spent waiting to acquire the mutex.",
        kind: FamilyKind::Histogram,
        samples: entries
            .iter()
            .map(|e| Sample {
                labels: labels(e),
                value: SampleValue::Histogram(HistogramValue {
                    sample_count: e.sampled_count,
                    sum: seconds(e.wait_total_nanos),
                    classic_buckets: classic_pairs(
                        &FAST_LADDER_NS,
                        e.classic_wait_buckets(&FAST_LADDER_NS),
                    ),
                    native_buckets: e.native_wait_buckets(NATIVE_SCHEMA),
                    zero_count: 0,
                }),
            })
            .collect(),
    });

    families.push(Family {
        name: "hotpath_mutex_acquire_seconds",
        help: "Time sampled acquisitions held the mutex.",
        kind: FamilyKind::Histogram,
        samples: entries
            .iter()
            .map(|e| Sample {
                labels: labels(e),
                value: SampleValue::Histogram(HistogramValue {
                    sample_count: e.sampled_count,
                    sum: seconds(e.acquire_total_nanos),
                    classic_buckets: classic_pairs(
                        &FAST_LADDER_NS,
                        e.classic_acquire_buckets(&FAST_LADDER_NS),
                    ),
                    native_buckets: e.native_acquire_buckets(NATIVE_SCHEMA),
                    zero_count: 0,
                }),
            })
            .collect(),
    });
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
fn collect_rw_locks(families: &mut Vec<Family>) {
    crate::lib_on::rw_locks::with_sorted_rw_lock_entries(|entries| {
        collect_rw_locks_entries(families, entries)
    });
}

fn collect_rw_locks_entries(
    families: &mut Vec<Family>,
    entries: &[&crate::lib_on::rw_locks::RwLockEntry],
) {
    use crate::lib_on::rw_locks::RwLockKind;

    let entries: Vec<&crate::lib_on::rw_locks::RwLockEntry> = entries
        .iter()
        .copied()
        .filter(|e| !e.key.is_empty())
        .collect();
    if entries.is_empty() {
        return;
    }

    const KINDS: [(RwLockKind, &str); 2] =
        [(RwLockKind::Read, "read"), (RwLockKind::Write, "write")];

    let labels = |e: &crate::lib_on::rw_locks::RwLockEntry, op: &str| {
        let mut labels = call_site_labels(e.key, e.label.as_deref(), e.iter);
        labels.push(("op", op.to_string()));
        labels
    };

    families.push(Family {
        name: "hotpath_rwlock_acquisitions_total",
        help: "Total lock acquisitions per rwlock call site and side, including acquisitions skipped by time sampling.",
        kind: FamilyKind::Counter,
        samples: entries
            .iter()
            .flat_map(|e| {
                KINDS.map(|(kind, op)| Sample {
                    labels: labels(e, op),
                    value: SampleValue::Scalar(e.count(kind) as f64),
                })
            })
            .collect(),
    });

    families.push(Family {
        name: "hotpath_rwlock_wait_seconds",
        help: "Time sampled acquisitions spent waiting to acquire the rwlock, per side.",
        kind: FamilyKind::Histogram,
        samples: entries
            .iter()
            .flat_map(|e| {
                KINDS.map(|(kind, op)| Sample {
                    labels: labels(e, op),
                    value: SampleValue::Histogram(HistogramValue {
                        sample_count: e.sampled_count(kind),
                        sum: seconds(e.wait_total_nanos(kind)),
                        classic_buckets: classic_pairs(
                            &FAST_LADDER_NS,
                            e.classic_wait_buckets(kind, &FAST_LADDER_NS),
                        ),
                        native_buckets: e.native_wait_buckets(kind, NATIVE_SCHEMA),
                        zero_count: 0,
                    }),
                })
            })
            .collect(),
    });

    families.push(Family {
        name: "hotpath_rwlock_acquire_seconds",
        help: "Time sampled acquisitions held the rwlock, per side.",
        kind: FamilyKind::Histogram,
        samples: entries
            .iter()
            .flat_map(|e| {
                KINDS.map(|(kind, op)| Sample {
                    labels: labels(e, op),
                    value: SampleValue::Histogram(HistogramValue {
                        sample_count: e.sampled_count(kind),
                        sum: seconds(e.acquire_total_nanos(kind)),
                        classic_buckets: classic_pairs(
                            &FAST_LADDER_NS,
                            e.classic_acquire_buckets(kind, &FAST_LADDER_NS),
                        ),
                        native_buckets: e.native_acquire_buckets(kind, NATIVE_SCHEMA),
                        zero_count: 0,
                    }),
                })
            })
            .collect(),
    });
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
fn collect_channels(families: &mut Vec<Family>) {
    crate::lib_on::channels::with_sorted_channel_entries(|entries| {
        collect_channels_entries(families, entries)
    });
}

fn collect_channels_entries(
    families: &mut Vec<Family>,
    entries: &[&crate::lib_on::channels::ChannelEntry],
) {
    let entries: Vec<&crate::lib_on::channels::ChannelEntry> = entries
        .iter()
        .copied()
        .filter(|e| !e.key.is_empty())
        .collect();
    if entries.is_empty() {
        return;
    }

    // Default-mode entries are keyed by call site plus payload type, so the
    // payload is part of the identity: a generic helper creating channels for
    // two payload types from one site yields two entries.
    let labels = |e: &crate::lib_on::channels::ChannelEntry| {
        let mut labels = call_site_labels(e.key, e.label.as_deref(), e.iter);
        labels.push(("type", e.channel_type.to_string()));
        labels.push(("payload", e.type_name.to_string()));
        labels
    };

    families.push(Family {
        name: "hotpath_channel_sent_total",
        help: "Messages sent per channel call site.",
        kind: FamilyKind::Counter,
        samples: entries
            .iter()
            .map(|e| Sample {
                labels: labels(e),
                value: SampleValue::Scalar(e.sent_count as f64),
            })
            .collect(),
    });

    families.push(Family {
        name: "hotpath_channel_received_total",
        help: "Messages received per channel call site.",
        kind: FamilyKind::Counter,
        samples: entries
            .iter()
            .map(|e| Sample {
                labels: labels(e),
                value: SampleValue::Scalar(e.received_count as f64),
            })
            .collect(),
    });

    families.push(Family {
        name: "hotpath_channel_instances_created_total",
        help: "Channel instances created at this call site since start.",
        kind: FamilyKind::Counter,
        samples: entries
            .iter()
            .map(|e| Sample {
                labels: labels(e),
                value: SampleValue::Scalar(e.instances as f64),
            })
            .collect(),
    });

    families.push(Family {
        name: "hotpath_channel_instances_closed_total",
        help: "Channel instances created at this call site that have closed.",
        kind: FamilyKind::Counter,
        samples: entries
            .iter()
            .map(|e| Sample {
                labels: labels(e),
                value: SampleValue::Scalar(e.closed_instances as f64),
            })
            .collect(),
    });

    let queue_sizes: Vec<Sample> = entries
        .iter()
        .filter_map(|e| {
            e.queue_size.map(|size| Sample {
                labels: labels(e),
                value: SampleValue::Scalar(size as f64),
            })
        })
        .collect();
    if !queue_sizes.is_empty() {
        families.push(Family {
            name: "hotpath_channel_queue_size",
            help: "Messages currently sent but not yet received.",
            kind: FamilyKind::Gauge,
            samples: queue_sizes,
        });
    }

    let max_queue_sizes: Vec<Sample> = entries
        .iter()
        .filter_map(|e| {
            e.max_queue_size.map(|size| Sample {
                labels: labels(e),
                value: SampleValue::Scalar(size as f64),
            })
        })
        .collect();
    if !max_queue_sizes.is_empty() {
        families.push(Family {
            name: "hotpath_channel_max_queue_size",
            help: "Since-start high-water mark of the queue size, not a windowed maximum.",
            kind: FamilyKind::Gauge,
            samples: max_queue_sizes,
        });
    }

    let delay: Vec<Sample> = entries
        .iter()
        .map(|e| Sample {
            labels: labels(e),
            value: SampleValue::Histogram(HistogramValue {
                sample_count: e.delay_sampled_count,
                sum: seconds(e.delay_total_nanos),
                classic_buckets: classic_pairs(
                    &FAST_LADDER_NS,
                    e.classic_delay_buckets(&FAST_LADDER_NS),
                ),
                native_buckets: e.native_delay_buckets(NATIVE_SCHEMA),
                zero_count: 0,
            }),
        })
        .collect();
    if !delay.is_empty() {
        families.push(Family {
            name: "hotpath_channel_delay_seconds",
            help: "Delay between send and sampled receive of a message.",
            kind: FamilyKind::Histogram,
            samples: delay,
        });
    }
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
fn collect_streams(families: &mut Vec<Family>) {
    let mut entries = crate::lib_on::streams::get_sorted_stream_stats();
    entries.retain(|e| !e.key.is_empty());
    if entries.is_empty() {
        return;
    }

    // Stream entries are keyed by call site plus item type - same identity
    // rule as channels.
    let labels = |e: &crate::lib_on::streams::StreamStats| {
        let mut labels = call_site_labels(e.key, e.label.as_deref(), e.iter);
        labels.push(("payload", e.type_name.to_string()));
        labels
    };

    families.push(Family {
        name: "hotpath_stream_items_total",
        help: "Items yielded per stream call site.",
        kind: FamilyKind::Counter,
        samples: entries
            .iter()
            .map(|e| Sample {
                labels: labels(e),
                value: SampleValue::Scalar(e.items_yielded as f64),
            })
            .collect(),
    });

    families.push(Family {
        name: "hotpath_stream_instances_created_total",
        help: "Stream instances created at this call site since start.",
        kind: FamilyKind::Counter,
        samples: entries
            .iter()
            .map(|e| Sample {
                labels: labels(e),
                value: SampleValue::Scalar(e.instances as f64),
            })
            .collect(),
    });

    families.push(Family {
        name: "hotpath_stream_instances_closed_total",
        help: "Stream instances created at this call site that have closed.",
        kind: FamilyKind::Counter,
        samples: entries
            .iter()
            .map(|e| Sample {
                labels: labels(e),
                value: SampleValue::Scalar(e.closed_instances as f64),
            })
            .collect(),
    });
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
fn collect_sql(families: &mut Vec<Family>) {
    crate::lib_on::sql::with_sorted_sql_entries(|entries| collect_sql_entries(families, entries));
}

fn collect_sql_entries(families: &mut Vec<Family>, entries: &[&crate::lib_on::sql::SqlEntry]) {
    if entries.is_empty() {
        return;
    }

    // Entries are keyed (route, source, normalized query), so these labels
    // expose the store's native granularity without adding series. The
    // normalized text itself is the query's identity - unlike the per-process
    // entry id it aggregates correctly across instances and restarts.
    let query_cap = *crate::output::MAX_LOG_LEN;
    let labels = |e: &crate::lib_on::sql::SqlEntry| {
        vec![
            ("query", query_label(&e.query, query_cap)),
            ("source", e.source.unwrap_or_default().to_string()),
            ("route", e.route.unwrap_or_default().to_string()),
        ]
    };

    families.push(Family {
        name: "hotpath_sql_queries_total",
        help: "Total executions of each tracked SQL query.",
        kind: FamilyKind::Counter,
        samples: entries
            .iter()
            .map(|e| Sample {
                labels: labels(e),
                value: SampleValue::Scalar(e.count as f64),
            })
            .collect(),
    });

    families.push(Family {
        name: "hotpath_sql_duration_seconds",
        help: "Duration of each tracked SQL query, labeled by its normalized text.",
        kind: FamilyKind::Histogram,
        samples: entries
            .iter()
            .map(|e| Sample {
                labels: labels(e),
                value: SampleValue::Histogram(HistogramValue {
                    sample_count: e.count,
                    sum: seconds(e.total_nanos),
                    classic_buckets: classic_pairs(
                        &SLOW_LADDER_NS,
                        e.classic_buckets(&SLOW_LADDER_NS),
                    ),
                    native_buckets: e.native_buckets(NATIVE_SCHEMA),
                    zero_count: 0,
                }),
            })
            .collect(),
    });
}

/// Normalized query text as a label value. Text over `cap` chars is truncated
/// with an FNV-1a hash of the full text appended, so two long queries sharing
/// a prefix cannot collapse into one series (duplicate label sets are invalid
/// exposition).
fn query_label(query: &str, cap: usize) -> String {
    if query.chars().count() <= cap {
        return query.to_string();
    }
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in query.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    let truncated: String = query.chars().take(cap).collect();
    format!("{}...{:016x}", truncated, hash)
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
fn collect_http(families: &mut Vec<Family>) {
    crate::lib_on::http::with_sorted_http_entries(|entries| {
        collect_http_entries(families, entries)
    });
}

fn collect_http_entries(families: &mut Vec<Family>, entries: &[&crate::lib_on::http::HttpEntry]) {
    if entries.is_empty() {
        return;
    }

    // Entries are keyed (route, source, endpoint), so all three labels are
    // needed to keep series distinct; aggregate with sum by (endpoint) in
    // PromQL for the per-endpoint view.
    let labels = |e: &crate::lib_on::http::HttpEntry| {
        vec![
            ("endpoint", e.endpoint.clone()),
            ("source", e.source.unwrap_or_default().to_string()),
            ("route", e.route.unwrap_or_default().to_string()),
        ]
    };

    families.push(Family {
        name: "hotpath_http_requests_total",
        help: "Total outbound HTTP requests per normalized endpoint.",
        kind: FamilyKind::Counter,
        samples: entries
            .iter()
            .map(|e| Sample {
                labels: labels(e),
                value: SampleValue::Scalar(e.count as f64),
            })
            .collect(),
    });

    families.push(Family {
        name: "hotpath_http_errors_total",
        help: "Outbound HTTP transport errors plus responses with status >= 400.",
        kind: FamilyKind::Counter,
        samples: entries
            .iter()
            .map(|e| Sample {
                labels: labels(e),
                value: SampleValue::Scalar(e.error_count as f64),
            })
            .collect(),
    });

    families.push(Family {
        name: "hotpath_http_duration_seconds",
        help: "Duration of outbound HTTP requests per normalized endpoint.",
        kind: FamilyKind::Histogram,
        samples: entries
            .iter()
            .map(|e| Sample {
                labels: labels(e),
                value: SampleValue::Histogram(HistogramValue {
                    sample_count: e.count,
                    sum: seconds(e.total_nanos),
                    classic_buckets: classic_pairs(
                        &SLOW_LADDER_NS,
                        e.classic_buckets(&SLOW_LADDER_NS),
                    ),
                    native_buckets: e.native_buckets(NATIVE_SCHEMA),
                    zero_count: 0,
                }),
            })
            .collect(),
    });
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
fn collect_server(families: &mut Vec<Family>) {
    crate::lib_on::server::with_sorted_server_entries(|entries| {
        collect_server_entries(families, entries)
    });
}

fn collect_server_entries(
    families: &mut Vec<Family>,
    entries: &[&crate::lib_on::server::ServerEntry],
) {
    if entries.is_empty() {
        return;
    }

    let route_labels = |e: &crate::lib_on::server::ServerEntry| vec![("route", e.route.clone())];

    families.push(Family {
        name: "hotpath_server_requests_total",
        help: "Total requests served per matched route template.",
        kind: FamilyKind::Counter,
        samples: entries
            .iter()
            .map(|e| Sample {
                labels: route_labels(e),
                value: SampleValue::Scalar(e.count as f64),
            })
            .collect(),
    });

    families.push(Family {
        name: "hotpath_server_responses_total",
        help: "Responses per route with a 4xx or 5xx status; other statuses are not classified.",
        kind: FamilyKind::Counter,
        samples: entries
            .iter()
            .flat_map(|e| {
                [("4xx", e.status_4xx), ("5xx", e.status_5xx)].map(|(class, count)| Sample {
                    labels: vec![("route", e.route.clone()), ("class", class.to_string())],
                    value: SampleValue::Scalar(count as f64),
                })
            })
            .collect(),
    });

    families.push(Family {
        name: "hotpath_server_duration_seconds",
        help: "Duration of served requests per route, until the response head is produced.",
        kind: FamilyKind::Histogram,
        samples: entries
            .iter()
            .map(|e| Sample {
                labels: route_labels(e),
                value: SampleValue::Histogram(HistogramValue {
                    sample_count: e.count,
                    sum: seconds(e.total_nanos),
                    classic_buckets: classic_pairs(
                        &SLOW_LADDER_NS,
                        e.classic_buckets(&SLOW_LADDER_NS),
                    ),
                    native_buckets: e.native_buckets(NATIVE_SCHEMA),
                    zero_count: 0,
                }),
            })
            .collect(),
    });

    families.push(Family {
        name: "hotpath_server_scoped_requests_total",
        help: "Completed requests that carried a route scope; the denominator for per-request SQL/HTTP rates.",
        kind: FamilyKind::Counter,
        samples: entries
            .iter()
            .map(|e| Sample {
                labels: route_labels(e),
                value: SampleValue::Scalar(e.scoped_count as f64),
            })
            .collect(),
    });

    families.push(Family {
        name: "hotpath_server_sql_calls_total",
        help: "SQL queries issued by route-scoped requests; divide by hotpath_server_scoped_requests_total for queries per request.",
        kind: FamilyKind::Counter,
        samples: entries
            .iter()
            .map(|e| Sample {
                labels: route_labels(e),
                value: SampleValue::Scalar(e.sql_calls as f64),
            })
            .collect(),
    });

    families.push(Family {
        name: "hotpath_server_http_calls_total",
        help: "Outbound HTTP requests issued by route-scoped requests; divide by hotpath_server_scoped_requests_total for requests per request.",
        kind: FamilyKind::Counter,
        samples: entries
            .iter()
            .map(|e| Sample {
                labels: route_labels(e),
                value: SampleValue::Scalar(e.http_calls as f64),
            })
            .collect(),
    });

    #[cfg(feature = "hotpath-alloc")]
    collect_server_alloc(families, entries);
}

/// Per-route memory; without the counting allocator every value would be 0.
#[cfg(feature = "hotpath-alloc")]
fn collect_server_alloc(
    families: &mut Vec<Family>,
    entries: &[&crate::lib_on::server::ServerEntry],
) {
    let route_labels = |e: &crate::lib_on::server::ServerEntry| vec![("route", e.route.clone())];

    families.push(Family {
        name: "hotpath_server_alloc_bytes_total",
        help: "Bytes allocated by route-scoped requests; divide by hotpath_server_scoped_requests_total for bytes per request.",
        kind: FamilyKind::Counter,
        samples: entries
            .iter()
            .map(|e| Sample {
                labels: route_labels(e),
                value: SampleValue::Scalar(e.alloc_bytes as f64),
            })
            .collect(),
    });

    families.push(Family {
        name: "hotpath_server_alloc_count_total",
        help: "Allocations made by route-scoped requests; divide by hotpath_server_scoped_requests_total for allocations per request.",
        kind: FamilyKind::Counter,
        samples: entries
            .iter()
            .map(|e| Sample {
                labels: route_labels(e),
                value: SampleValue::Scalar(e.alloc_count as f64),
            })
            .collect(),
    });

    // Routes without a scoped request have no per-request distribution.
    let samples: Vec<Sample> = entries
        .iter()
        .filter(|e| e.scoped_count > 0)
        .map(|e| Sample {
            labels: route_labels(e),
            value: SampleValue::Histogram(HistogramValue {
                sample_count: e.scoped_count,
                sum: e.alloc_bytes as f64,
                classic_buckets: classic_pairs_units(
                    ALLOC_LADDER_BYTES,
                    e.alloc_classic_buckets(ALLOC_LADDER_BYTES),
                ),
                native_buckets: e.alloc_native_buckets(NATIVE_SCHEMA),
                zero_count: e.alloc_zero_count(),
            }),
        })
        .collect();
    if !samples.is_empty() {
        families.push(Family {
            name: "hotpath_server_alloc_bytes",
            help: "Bytes allocated per route-scoped request; values clamp at 1GB.",
            kind: FamilyKind::Histogram,
            samples,
        });
    }
}

#[cfg(feature = "threads")]
#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
fn collect_threads(families: &mut Vec<Family>) {
    let Some(threads) = crate::lib_on::threads::get_threads_raw() else {
        return;
    };
    if threads.metrics.is_empty() {
        return;
    }

    // Per-thread series cover only live sampled threads: the rows joined in
    // from the allocation registry (exited threads) would otherwise grow the
    // exported label sets without bound under thread churn. Prometheus
    // staleness handles a dead thread's series disappearing; the bytes it
    // allocated stay visible in the process-level totals below.
    let live = &threads.metrics[..threads.live_count];

    let labels = |m: &crate::json::ThreadMetrics| {
        vec![("name", m.name.clone()), ("tid", m.os_tid.to_string())]
    };

    families.push(Family {
        name: "hotpath_threads",
        help: "Threads in the most recent monitor sample.",
        kind: FamilyKind::Gauge,
        samples: vec![Sample {
            labels: vec![],
            value: SampleValue::Scalar(threads.live_count as f64),
        }],
    });

    if let Some(rss) = threads.rss_bytes {
        families.push(Family {
            name: "hotpath_rss_bytes",
            help: "Resident set size of the process.",
            kind: FamilyKind::Gauge,
            samples: vec![Sample {
                labels: vec![],
                value: SampleValue::Scalar(rss as f64),
            }],
        });
    }

    let per_thread_samples = |value: &dyn Fn(&crate::json::ThreadMetrics) -> Option<f64>| {
        live.iter()
            .filter_map(|m| {
                value(m).map(|v| Sample {
                    labels: labels(m),
                    value: SampleValue::Scalar(v),
                })
            })
            .collect::<Vec<_>>()
    };

    let cpu_percent = per_thread_samples(&|m| m.cpu_percent);
    if !cpu_percent.is_empty() {
        families.push(Family {
            name: "hotpath_thread_cpu_percent",
            help: "CPU usage of each sampled thread over the last monitor interval.",
            kind: FamilyKind::Gauge,
            samples: cpu_percent,
        });
    }

    let cpu_percent_max = per_thread_samples(&|m| m.cpu_percent_max);
    if !cpu_percent_max.is_empty() {
        families.push(Family {
            name: "hotpath_thread_cpu_percent_max",
            help: "Since-start peak CPU usage of each sampled thread, not a windowed maximum.",
            kind: FamilyKind::Gauge,
            samples: cpu_percent_max,
        });
    }

    let cpu_percent_avg = per_thread_samples(&|m| m.cpu_percent_avg);
    if !cpu_percent_avg.is_empty() {
        families.push(Family {
            name: "hotpath_thread_cpu_percent_avg",
            help: "Lifetime average CPU usage of each sampled thread.",
            kind: FamilyKind::Gauge,
            samples: cpu_percent_avg,
        });
    }

    families.push(Family {
        name: "hotpath_thread_cpu_seconds_total",
        help: "CPU time consumed by each sampled thread, split by mode.",
        kind: FamilyKind::Counter,
        samples: live
            .iter()
            .flat_map(|m| {
                [("user", m.cpu_user), ("sys", m.cpu_sys)].map(|(mode, seconds)| Sample {
                    labels: {
                        let mut labels = labels(m);
                        labels.push(("mode", mode.to_string()));
                        labels
                    },
                    value: SampleValue::Scalar(seconds),
                })
            })
            .collect(),
    });

    let alloc_bytes = per_thread_samples(&|m| m.alloc_bytes.map(|b| b as f64));
    if !alloc_bytes.is_empty() {
        families.push(Family {
            name: "hotpath_thread_alloc_bytes_total",
            help: "Bytes allocated by each thread (requires hotpath-alloc).",
            kind: FamilyKind::Counter,
            samples: alloc_bytes,
        });
    }

    let dealloc_bytes = per_thread_samples(&|m| m.dealloc_bytes.map(|b| b as f64));
    if !dealloc_bytes.is_empty() {
        families.push(Family {
            name: "hotpath_thread_dealloc_bytes_total",
            help: "Bytes deallocated by each thread (requires hotpath-alloc).",
            kind: FamilyKind::Counter,
            samples: dealloc_bytes,
        });
    }

    // Process-level totals span every thread ever tracked, exited included,
    // plus the overflow bytes from threads the capped registry could not
    // slot: one bounded series preserving the bytes that leave the
    // per-thread view when a thread's series goes stale.
    let (total_alloc, total_dealloc, has_alloc_data) = threads.metrics.iter().fold(
        (
            threads.overflow_alloc_bytes,
            threads.overflow_dealloc_bytes,
            false,
        ),
        |(alloc, dealloc, has), m| {
            (
                alloc + m.alloc_bytes.unwrap_or(0),
                dealloc + m.dealloc_bytes.unwrap_or(0),
                has || m.alloc_bytes.is_some(),
            )
        },
    );
    if has_alloc_data {
        families.push(Family {
            name: "hotpath_alloc_bytes_total",
            help: "Bytes allocated by the process across all threads, exited included (requires hotpath-alloc).",
            kind: FamilyKind::Counter,
            samples: vec![Sample {
                labels: vec![],
                value: SampleValue::Scalar(total_alloc as f64),
            }],
        });
        families.push(Family {
            name: "hotpath_dealloc_bytes_total",
            help: "Bytes deallocated by the process across all threads, exited included (requires hotpath-alloc).",
            kind: FamilyKind::Counter,
            samples: vec![Sample {
                labels: vec![],
                value: SampleValue::Scalar(total_dealloc as f64),
            }],
        });
    }
}

#[cfg(feature = "tokio")]
#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
fn collect_tokio_runtime(families: &mut Vec<Family>) {
    let Some(rt) = crate::lib_on::tokio_runtime::get_runtime_json() else {
        return;
    };

    let mut gauge = |name: &'static str, help: &'static str, value: f64| {
        families.push(Family {
            name,
            help,
            kind: FamilyKind::Gauge,
            samples: vec![Sample {
                labels: vec![],
                value: SampleValue::Scalar(value),
            }],
        });
    };

    gauge(
        "hotpath_tokio_workers",
        "Worker threads of the sampled tokio runtime.",
        rt.num_workers as f64,
    );
    gauge(
        "hotpath_tokio_alive_tasks",
        "Tasks currently alive in the sampled tokio runtime.",
        rt.num_alive_tasks as f64,
    );
    gauge(
        "hotpath_tokio_global_queue_depth",
        "Tasks waiting in the runtime's global injection queue.",
        rt.global_queue_depth as f64,
    );
    if let Some(v) = rt.num_blocking_threads {
        gauge(
            "hotpath_tokio_blocking_threads",
            "Threads in the blocking pool.",
            v as f64,
        );
    }
    if let Some(v) = rt.num_idle_blocking_threads {
        gauge(
            "hotpath_tokio_idle_blocking_threads",
            "Idle threads in the blocking pool.",
            v as f64,
        );
    }
    if let Some(v) = rt.blocking_queue_depth {
        gauge(
            "hotpath_tokio_blocking_queue_depth",
            "Tasks waiting for the blocking pool.",
            v as f64,
        );
    }

    let mut counter = |name: &'static str, help: &'static str, value: Option<u64>| {
        if let Some(value) = value {
            families.push(Family {
                name,
                help,
                kind: FamilyKind::Counter,
                samples: vec![Sample {
                    labels: vec![],
                    value: SampleValue::Scalar(value as f64),
                }],
            });
        }
    };

    counter(
        "hotpath_tokio_spawned_tasks_total",
        "Tasks spawned on the runtime since start.",
        rt.spawned_tasks_count,
    );
    counter(
        "hotpath_tokio_remote_schedules_total",
        "Tasks scheduled from outside the runtime since start.",
        rt.remote_schedule_count,
    );
    counter(
        "hotpath_tokio_io_fd_registered_total",
        "File descriptors registered with the io driver.",
        rt.io_driver_fd_registered_count,
    );
    counter(
        "hotpath_tokio_io_fd_deregistered_total",
        "File descriptors deregistered from the io driver.",
        rt.io_driver_fd_deregistered_count,
    );
    counter(
        "hotpath_tokio_io_ready_events_total",
        "Readiness events delivered by the io driver.",
        rt.io_driver_ready_count,
    );

    if rt.workers.is_empty() {
        return;
    }

    let worker_labels = |index: usize| vec![("worker", index.to_string())];

    families.push(Family {
        name: "hotpath_tokio_worker_parks_total",
        help: "Times each worker parked (went idle).",
        kind: FamilyKind::Counter,
        samples: rt
            .workers
            .iter()
            .map(|w| Sample {
                labels: worker_labels(w.index),
                value: SampleValue::Scalar(w.park_count as f64),
            })
            .collect(),
    });

    families.push(Family {
        name: "hotpath_tokio_worker_busy_seconds_total",
        help: "Time each worker spent executing tasks.",
        kind: FamilyKind::Counter,
        samples: rt
            .workers
            .iter()
            .map(|w| Sample {
                labels: worker_labels(w.index),
                value: SampleValue::Scalar(w.busy_duration_ms as f64 / 1e3),
            })
            .collect(),
    });

    let worker_counter =
        |families: &mut Vec<Family>,
         name: &'static str,
         help: &'static str,
         value: &dyn Fn(&crate::json::JsonRuntimeWorker) -> Option<u64>| {
            let samples: Vec<Sample> = rt
                .workers
                .iter()
                .filter_map(|w| {
                    value(w).map(|v| Sample {
                        labels: worker_labels(w.index),
                        value: SampleValue::Scalar(v as f64),
                    })
                })
                .collect();
            if !samples.is_empty() {
                families.push(Family {
                    name,
                    help,
                    kind: FamilyKind::Counter,
                    samples,
                });
            }
        };

    worker_counter(
        families,
        "hotpath_tokio_worker_polls_total",
        "Tasks polled by each worker.",
        &|w| w.poll_count,
    );
    worker_counter(
        families,
        "hotpath_tokio_worker_steals_total",
        "Tasks each worker stole from other workers' queues.",
        &|w| w.steal_count,
    );

    let local_depths: Vec<Sample> = rt
        .workers
        .iter()
        .filter_map(|w| {
            w.local_queue_depth.map(|v| Sample {
                labels: worker_labels(w.index),
                value: SampleValue::Scalar(v as f64),
            })
        })
        .collect();
    if !local_depths.is_empty() {
        families.push(Family {
            name: "hotpath_tokio_worker_local_queue_depth",
            help: "Tasks waiting in each worker's local queue.",
            kind: FamilyKind::Gauge,
            samples: local_depths,
        });
    }
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
fn collect_gauges(families: &mut Vec<Family>) {
    let entries = crate::lib_on::debug::get_sorted_debug_gauge_entries();
    if entries.is_empty() {
        return;
    }

    // Gauge identity is the key alone (`source` is whichever call site
    // created the entry first, so it would churn series across runs when one
    // key is updated from several locations).
    let labels = |e: &crate::lib_on::debug::gauge::GaugeEntry| vec![("key", e.key.to_string())];

    families.push(Family {
        name: "hotpath_gauge",
        help: "Current value of each gauge! entry.",
        kind: FamilyKind::Gauge,
        samples: entries
            .iter()
            .map(|e| Sample {
                labels: labels(e),
                value: SampleValue::Scalar(e.current_value),
            })
            .collect(),
    });

    families.push(Family {
        name: "hotpath_gauge_min",
        help: "Since-start minimum of each gauge! entry.",
        kind: FamilyKind::Gauge,
        samples: entries
            .iter()
            .map(|e| Sample {
                labels: labels(e),
                value: SampleValue::Scalar(e.min_value),
            })
            .collect(),
    });

    families.push(Family {
        name: "hotpath_gauge_max",
        help: "Since-start maximum of each gauge! entry.",
        kind: FamilyKind::Gauge,
        samples: entries
            .iter()
            .map(|e| Sample {
                labels: labels(e),
                value: SampleValue::Scalar(e.max_value),
            })
            .collect(),
    });

    families.push(Family {
        name: "hotpath_gauge_updates_total",
        help: "Updates applied to each gauge! entry.",
        kind: FamilyKind::Counter,
        samples: entries
            .iter()
            .map(|e| Sample {
                labels: labels(e),
                value: SampleValue::Scalar(e.update_count as f64),
            })
            .collect(),
    });
}

/// Futures mirror [`call_site_labels`] minus `iter` (one entry per id, never
/// per instantiation): the id is the identity verbatim - `file:line:column`
/// for `future!` sites, the function path for name-based ids (`#[future_fn]`).
fn future_labels(id: &'static str, label: Option<&str>) -> Vec<(&'static str, String)> {
    vec![
        ("source", id.to_string()),
        ("label", label.unwrap_or_default().to_string()),
    ]
}

/// Shared call-site identity labels for entries keyed by wrapper macro call
/// site (locks, channels, streams, io) - the store's entry identity verbatim,
/// so distinct entries can never collapse into duplicate series. `source` is
/// the column-including `file:line:column` key (the column keeps two
/// invocations on one physical line distinct); `iter` is the instantiation
/// index for call sites that produce one entry per instantiation; `label` is
/// the user's label verbatim.
fn call_site_labels(key: &str, label: Option<&str>, iter: u32) -> Vec<(&'static str, String)> {
    vec![
        ("source", key.to_string()),
        ("label", label.unwrap_or_default().to_string()),
        ("iter", iter.to_string()),
    ]
}

/// Ladder boundaries (ns) zipped with their cumulative counts into the
/// `(upper bound seconds, count)` pairs the family model stores.
fn classic_pairs(ladder: &[u64], counts: Vec<u64>) -> Vec<(f64, u64)> {
    ladder
        .iter()
        .zip(counts)
        .map(|(&boundary_ns, cumulative)| (seconds(boundary_ns), cumulative))
        .collect()
}

/// [`classic_pairs`] for ladders already in base units (bytes, counts): the
/// boundary is exported as-is.
fn classic_pairs_units(ladder: &[u64], counts: Vec<u64>) -> Vec<(f64, u64)> {
    ladder
        .iter()
        .zip(counts)
        .map(|(&boundary, cumulative)| (boundary as f64, cumulative))
        .collect()
}

/// f64 `Display` never uses scientific notation, which the exposition format
/// tolerates but some parsers of `le` label values do not.
fn seconds(ns: u64) -> f64 {
    ns as f64 / 1e9
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
fn respond_bytes(request: Request, body: Vec<u8>, content_type: &str) {
    let mut response = Response::from_data(body);
    response.add_header(
        Header::from_bytes(b"Content-Type".as_slice(), content_type.as_bytes()).unwrap(),
    );
    let _ = request.respond(response);
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
fn respond_text(request: Request, code: u16, msg: &str) {
    let mut response = Response::from_string(msg).with_status_code(code);
    response.add_header(
        Header::from_bytes(
            b"Content-Type".as_slice(),
            b"text/plain; charset=utf-8".as_slice(),
        )
        .unwrap(),
    );
    let _ = request.respond(response);
}

#[cfg(test)]
mod tests {
    use crate::prometheus_server::{
        accepts_protobuf, check_auth_with_bearer, parse_ladder, query_label, seconds,
    };

    #[test]
    fn parse_ladder_accepts_ascending_seconds() {
        assert_eq!(
            parse_ladder("0.000001, 0.00002,0.5,1000").unwrap(),
            vec![1_000, 20_000, 500_000_000, 1_000_000_000_000]
        );
        assert_eq!(parse_ladder("1e-3").unwrap(), vec![1_000_000]);
    }

    #[test]
    fn parse_ladder_rejects_invalid_input() {
        for raw in [
            "",
            "abc",
            "0.001,",
            "0.001,,0.01",
            "-0.001",
            "0",
            "inf",
            "NaN",
            "0.0000000001",
            "1001",
            "0.01,0.001",
            "0.001,0.001",
            "0.0000000010,0.0000000014",
        ] {
            assert!(parse_ladder(raw).is_err(), "`{raw}` should be rejected");
        }
    }

    #[test]
    fn future_labels_use_id_verbatim() {
        use crate::prometheus_server::future_labels;
        let a = future_labels("src/main.rs:12:34", None);
        let b = future_labels("src/main.rs:12:60", None);
        assert_eq!(a[0], ("source", "src/main.rs:12:34".to_string()));
        assert_ne!(a, b, "same-line future! sites must stay distinct");

        let named = future_labels("my_module::my_future_fn", Some("x"));
        assert_eq!(named[0].1, "my_module::my_future_fn");
    }

    #[test]
    fn call_site_labels_mirror_entry_identity() {
        use crate::prometheus_server::call_site_labels;
        let a = call_site_labels("src/app.rs:10:5", None, 0);
        assert_eq!(a[0], ("source", "src/app.rs:10:5".to_string()));
        assert_eq!(a[2], ("iter", "0".to_string()));

        // The column-including key keeps same-line call sites distinct,
        // repeated instantiations differ by iter, and the user label stays
        // verbatim (no suffix encoding that could alias two entries).
        let b = call_site_labels("src/app.rs:10:30", None, 0);
        assert_ne!(a, b);
        let worker_2 = call_site_labels("src/app.rs:10:5", Some("worker-2"), 0);
        let worker_iter = call_site_labels("src/app.rs:10:5", Some("worker"), 1);
        assert_ne!(worker_2, worker_iter);
        assert_eq!(worker_iter[1], ("label", "worker".to_string()));
    }

    #[test]
    fn query_label_truncation_stays_unique() {
        assert_eq!(query_label("SELECT 1", 100), "SELECT 1");

        let a = query_label(&format!("SELECT {}", "a".repeat(200)), 50);
        let b = query_label(&format!("SELECT {}", "a".repeat(201)), 50);
        assert_eq!(a.chars().count(), 50 + 3 + 16);
        assert_ne!(a, b, "shared-prefix long queries must stay distinct");
        assert_eq!(a[..50], b[..50]);
    }

    #[test]
    fn seconds_formats_without_exponent() {
        assert_eq!(seconds(250).to_string(), "0.00000025");
        assert_eq!(seconds(1_000_000_000).to_string(), "1");
        assert_eq!(seconds(3_000_000_000).to_string(), "3");
    }

    #[test]
    fn bearer_prefix_accepted() {
        assert!(check_auth_with_bearer(Some("secret"), Some("secret")));
        assert!(check_auth_with_bearer(
            Some("secret"),
            Some("Bearer secret")
        ));
        assert!(!check_auth_with_bearer(
            Some("secret"),
            Some("Bearer wrong")
        ));
        assert!(!check_auth_with_bearer(Some("secret"), None));
        assert!(check_auth_with_bearer(None, None));
    }

    #[test]
    fn protobuf_negotiation() {
        assert!(accepts_protobuf(
            "application/vnd.google.protobuf;proto=io.prometheus.client.MetricFamily;encoding=delimited,text/plain;version=0.0.4;q=0.5"
        ));
        assert!(!accepts_protobuf("text/plain; version=0.0.4"));
        assert!(!accepts_protobuf(""));
    }
}

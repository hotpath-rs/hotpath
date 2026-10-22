//! Drives every instrumented subsystem of `hotpath` a fixed number of times so
//! the `hotpath-meta` report measures the profiler's own hot paths under load.
//! Every phase is count-driven and sleep-free, so call counts are identical
//! between runs and a diff of two reports isolates timing changes. Changing the
//! constants below invalidates comparability with previously uploaded reports.
//!
//! An in-process axum server (`hotpath::axum!`) backed by an instrumented
//! Diesel SQLite connection is called through an instrumented ureq agent, so
//! the SQL, HTTP, server and per-route function sections hold data too.
//!
//! With `hotpath-prometheus` on, the exporter is scraped too, in both the text
//! and protobuf encodings, so the meta report covers every way a live session
//! reads data out of the profiler.
//!
//! Run with:
//!   cargo run --release -p test-all-features --example benchmark_meta --features hotpath-alloc,hotpath-alloc-meta,hotpath-prometheus

use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::extract::{Path, State};
use axum::routing::get;
use axum::Router;
use diesel::prelude::*;
use diesel::sql_types::Integer;
use futures::StreamExt;

const SYNC_RUNS: u64 = 1_250_000;
const ALLOC_RUNS: u64 = 625_000;
const ASYNC_RUNS: u64 = 375_000;
const BLOCK_RUNS: u64 = 625_000;
const FUTURE_RUNS: u64 = 375_000;
const STREAM_ITEMS: u64 = 1_250_000;
const CHANNEL_MSGS: u64 = 375_000;
const CHANNEL_INSTANCES: u64 = 50_000;
const LOCK_RUNS: u64 = 625_000;
const IO_RUNS: u64 = 375_000;
const DEBUG_RUNS: u64 = 62_500;
const THREAD_COUNT: usize = 4;
const THREAD_RUNS: u64 = 250_000;
const WEB_REQUESTS: u64 = 2_000;
const WEB_ITEMS: i32 = 100;
const METRICS_REQUESTS: u64 = 120;
#[cfg(feature = "hotpath-prometheus")]
const PROMETHEUS_REQUESTS: u64 = 1_000;

#[hotpath::measure]
fn sync_noop(v: u64) -> u64 {
    std::hint::black_box(v).wrapping_mul(31)
}

#[hotpath::measure]
fn sync_alloc(v: u64) -> usize {
    let buf: Vec<u64> = (0..8).map(|i| i ^ v).collect();
    std::hint::black_box(&buf);
    buf.len()
}

#[hotpath::measure]
async fn async_noop(v: u64) -> u64 {
    std::hint::black_box(v).wrapping_add(7)
}

struct Accumulator {
    total: u64,
}

#[hotpath::measure_all]
impl Accumulator {
    fn new() -> Self {
        Self { total: 0 }
    }

    fn add(&mut self, v: u64) {
        self.total = self.total.wrapping_add(v);
    }
}

#[derive(Clone)]
struct WebState {
    conn: Arc<Mutex<SqliteConnection>>,
}

#[hotpath::measure]
fn load_item(conn: &mut SqliteConnection, id: i32) -> usize {
    diesel::sql_query("SELECT id, name FROM items WHERE id = ?")
        .bind::<Integer, _>(id)
        .execute(conn)
        .expect("load_item")
}

#[hotpath::measure]
fn count_items(conn: &mut SqliteConnection) -> usize {
    diesel::sql_query("SELECT COUNT(*) FROM items")
        .execute(conn)
        .expect("count_items")
}

async fn get_item(State(state): State<WebState>, Path(id): Path<i32>) -> String {
    let rows = load_item(&mut state.conn.lock().expect("poisoned"), id);
    format!("item {id}: {rows}")
}

// Two queries under one route, so the server section reports 2 SQL/req here.
async fn get_summary(State(state): State<WebState>, Path(id): Path<i32>) -> String {
    let mut conn = state.conn.lock().expect("poisoned");
    let rows = load_item(&mut conn, id);
    let total = count_items(&mut conn);
    format!("item {id}: {rows} of {total}")
}

/// Serves the instrumented router on a fresh port and returns its base URL.
async fn start_web_server() -> String {
    hotpath::instrument_diesel_sql();
    let mut conn = SqliteConnection::establish(":memory:").expect("sqlite");
    diesel::sql_query("CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
        .execute(&mut conn)
        .expect("create table");
    diesel::sql_query(
        "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < ?) \
         INSERT INTO items (id, name) SELECT x, 'item-' || x FROM c",
    )
    .bind::<Integer, _>(WEB_ITEMS)
    .execute(&mut conn)
    .expect("seed items");

    let router = hotpath::axum!(Router::new()
        .route("/items/{id}", get(get_item))
        .route("/summaries/{id}", get(get_summary))
        .with_state(WebState {
            conn: Arc::new(Mutex::new(conn)),
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("local addr").port();
    tokio::spawn(async move {
        axum::serve(listener, router).await.expect("axum server");
    });
    format!("http://127.0.0.1:{port}")
}

/// Every tenth request hits a missing route, so the HTTP and server sections
/// carry 404s next to the successful requests.
fn call_web_server(base: &str) {
    let agent: ureq::Agent = hotpath::http!(ureq::Agent::config_builder())
        .build()
        .new_agent();
    for i in 0..WEB_REQUESTS {
        let id = (i % WEB_ITEMS as u64) as i32 + 1;
        match i % 10 {
            9 => {
                let result = agent.get(format!("{base}/missing/{id}")).call();
                assert!(matches!(result, Err(ureq::Error::StatusCode(404))));
            }
            n if n % 2 == 0 => {
                agent
                    .get(format!("{base}/items/{id}"))
                    .call()
                    .expect("items request");
            }
            _ => {
                agent
                    .get(format!("{base}/summaries/{id}"))
                    .call()
                    .expect("summaries request");
            }
        }
    }
}

#[tokio::main]
#[hotpath::main]
async fn main() {
    let overall = Instant::now();

    phase("functions", SYNC_RUNS, || {
        let mut acc = Accumulator::new();
        for i in 0..SYNC_RUNS {
            acc.add(sync_noop(i));
        }
        std::hint::black_box(acc.total);
    });

    phase("functions_alloc", ALLOC_RUNS, || {
        let mut total = 0usize;
        for i in 0..ALLOC_RUNS {
            total += sync_alloc(i);
        }
        std::hint::black_box(total);
    });

    phase("measure_block", BLOCK_RUNS, || {
        let mut total = 0u64;
        for i in 0..BLOCK_RUNS {
            total = total.wrapping_add(hotpath::measure_block!("bench_block", {
                std::hint::black_box(i).wrapping_mul(3)
            }));
        }
        std::hint::black_box(total);
    });

    phase_async("functions_async", ASYNC_RUNS, async {
        let mut total = 0u64;
        for i in 0..ASYNC_RUNS {
            total = total.wrapping_add(async_noop(i).await);
        }
        std::hint::black_box(total);
    })
    .await;

    phase_async("futures", FUTURE_RUNS, async {
        let mut total = 0u64;
        for i in 0..FUTURE_RUNS {
            total = total.wrapping_add(
                hotpath::future!(
                    async move { std::hint::black_box(i) },
                    label = "bench-future"
                )
                .await,
            );
        }
        std::hint::black_box(total);
    })
    .await;

    phase_async("streams", STREAM_ITEMS, async {
        let mut s = hotpath::stream!(
            futures::stream::iter(0..STREAM_ITEMS),
            label = "bench-items"
        );
        let mut total = 0u64;
        while let Some(v) = s.next().await {
            total = total.wrapping_add(v);
        }
        std::hint::black_box(total);
    })
    .await;

    phase_async("channels_tokio", CHANNEL_MSGS, async {
        let (tx, mut rx) = hotpath::channel!(
            tokio::sync::mpsc::channel::<u64>(64),
            label = "bench-tokio-mpsc"
        );
        let producer = tokio::spawn(async move {
            for i in 0..CHANNEL_MSGS {
                tx.send(i).await.expect("receiver dropped");
            }
        });
        let mut total = 0u64;
        while let Some(v) = rx.recv().await {
            total = total.wrapping_add(v);
        }
        producer.await.expect("producer panicked");
        std::hint::black_box(total);
    })
    .await;

    phase("channels_std", CHANNEL_MSGS, || {
        let (tx, rx) = hotpath::channel!(
            std::sync::mpsc::sync_channel::<u64>(64),
            capacity = 64,
            label = "bench-std-mpsc"
        );
        let producer = std::thread::spawn(move || {
            for i in 0..CHANNEL_MSGS {
                tx.send(i).expect("receiver dropped");
            }
        });
        let mut total = 0u64;
        while let Ok(v) = rx.recv() {
            total = total.wrapping_add(v);
        }
        producer.join().expect("producer panicked");
        std::hint::black_box(total);
    });

    // Aggregated mode: a fresh channel per iteration at one call site, so the
    // per-instance registration path is measured rather than the one-time
    // entry construction.
    phase("channels_aggregated", CHANNEL_INSTANCES, || {
        for i in 0..CHANNEL_INSTANCES {
            let (tx, rx) = hotpath::channel!(
                std::sync::mpsc::sync_channel::<u64>(4),
                capacity = 4,
                label = "bench-per-request"
            );
            tx.send(i).expect("receiver dropped");
            drop(tx);
            while let Ok(v) = rx.recv() {
                std::hint::black_box(v);
            }
        }
    });

    phase("mutex_std", LOCK_RUNS, || {
        let mutex = hotpath::mutex!(std::sync::Mutex::new(0u64), label = "bench-std-mutex");
        for i in 0..LOCK_RUNS {
            let mut v = mutex.lock().expect("poisoned");
            *v = v.wrapping_add(i);
        }
    });

    phase("rw_lock_std", LOCK_RUNS, || {
        let lock = hotpath::rw_lock!(std::sync::RwLock::new(0u64), label = "bench-std-rw-lock");
        for i in 0..LOCK_RUNS {
            if i % 10 == 0 {
                let mut w = lock.write().expect("poisoned");
                *w = w.wrapping_add(i);
            } else {
                let r = lock.read().expect("poisoned");
                std::hint::black_box(*r);
            }
        }
    });

    phase_async("mutex_tokio", LOCK_RUNS, async {
        let mutex = hotpath::mutex!(tokio::sync::Mutex::new(0u64), label = "bench-tokio-mutex");
        for i in 0..LOCK_RUNS {
            let mut v = mutex.lock().await;
            *v = v.wrapping_add(i);
        }
    })
    .await;

    phase_async("rw_lock_tokio", LOCK_RUNS, async {
        let lock = hotpath::rw_lock!(
            tokio::sync::RwLock::new(0u64),
            label = "bench-tokio-rw-lock"
        );
        for i in 0..LOCK_RUNS {
            if i % 10 == 0 {
                let mut w = lock.write().await;
                *w = w.wrapping_add(i);
            } else {
                let r = lock.read().await;
                std::hint::black_box(*r);
            }
        }
    })
    .await;

    phase("io", IO_RUNS, || {
        let mut cursor = hotpath::io!(std::io::Cursor::new(Vec::<u8>::new()), label = "bench-io");
        let chunk = [7u8; 64];
        for _ in 0..IO_RUNS {
            cursor.write_all(&chunk).expect("write failed");
        }
        cursor.seek(SeekFrom::Start(0)).expect("seek failed");
        let mut buf = [0u8; 64];
        for _ in 0..IO_RUNS {
            cursor.read_exact(&mut buf).expect("read failed");
        }
        std::hint::black_box(buf[0]);
    });

    phase("debug", DEBUG_RUNS, || {
        for i in 0..DEBUG_RUNS {
            let v = hotpath::dbg!(i);
            hotpath::val!("bench-state").set(&v);
            hotpath::gauge!("bench-gauge").set(v as f64);
        }
    });

    // Cross-thread load: every producer owns its own measurement queue, so the
    // background worker sweeps `THREAD_COUNT + 1` queues per tick.
    phase("threads", THREAD_COUNT as u64 * THREAD_RUNS, || {
        let shared = Arc::new(hotpath::mutex!(
            std::sync::Mutex::new(0u64),
            label = "bench-shared-mutex"
        ));
        let handles: Vec<_> = (0..THREAD_COUNT)
            .map(|_| {
                let shared = Arc::clone(&shared);
                std::thread::spawn(move || {
                    let mut acc = Accumulator::new();
                    for i in 0..THREAD_RUNS {
                        acc.add(sync_noop(i));
                        if i % 1_000 == 0 {
                            let mut v = shared.lock().expect("poisoned");
                            *v = v.wrapping_add(1);
                        }
                    }
                    acc.total
                })
            })
            .collect();
        for h in handles {
            std::hint::black_box(h.join().expect("worker panicked"));
        }
    });

    // The blocking ureq client runs off the runtime so the axum server keeps
    // every worker it needs.
    let base = start_web_server().await;
    phase_async("web", WEB_REQUESTS, async move {
        tokio::task::spawn_blocking(move || call_web_server(&base))
            .await
            .expect("web client panicked");
    })
    .await;

    // Drives the metrics server and the worker snapshot round trip that the TUI
    // exercises in a live session.
    phase("metrics_server", METRICS_REQUESTS, scrape_metrics);

    // The exporter renders every section from a worker snapshot on each scrape,
    // and both encodings walk the same data through different serializers.
    #[cfg(feature = "hotpath-prometheus")]
    phase("prometheus", PROMETHEUS_REQUESTS, scrape_prometheus);

    println!("benchmark_meta: total {:?}", overall.elapsed());
}

/// Every section the TUI polls, plus the cheap envelopes it polls most often.
const METRICS_ROUTES: &[&str] = &[
    "/functions_timing",
    "/functions_alloc",
    "/channels",
    "/streams",
    "/futures",
    "/rw_locks",
    "/mutexes",
    "/sql",
    "/http",
    "/server",
    "/io",
    "/debug",
    "/threads",
    "/profiler_status",
];
// `/tokio_runtime` is deliberately absent: it needs `--cfg tokio_unstable` and a
// registered runtime, so here it would only ever measure a 404.

/// Sections whose per-entry logs the TUI fetches when a row is expanded. The
/// `{id}` is filled from the section listing so the request hits an entry that
/// exists and serializes its logs, instead of a 404 that measures nothing.
const LOG_ROUTES: &[(&str, &str)] = &[
    ("/functions_timing", "/functions_timing/{id}/logs"),
    ("/functions_alloc", "/functions_alloc/{id}/logs"),
    ("/channels", "/channels/{id}/logs"),
    ("/streams", "/streams/{id}/logs"),
    ("/futures", "/futures/{id}/logs"),
];

fn metrics_port() -> u16 {
    std::env::var("HOTPATH_METRICS_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6770)
}

fn scrape_metrics() {
    let port = metrics_port();

    let mut log_routes = Vec::new();
    for (section, template) in LOG_ROUTES {
        match scrape_route(port, section, None) {
            Ok(body) => {
                if let Some(id) = first_id(&body) {
                    log_routes.push(template.replace("{id}", &id.to_string()));
                }
            }
            Err(e) => {
                eprintln!("benchmark_meta: metrics request {section} failed: {e}");
                return;
            }
        }
    }

    for i in 0..METRICS_REQUESTS {
        let i = i as usize;
        let route = if i % 4 == 3 && !log_routes.is_empty() {
            log_routes[i % log_routes.len()].as_str()
        } else {
            METRICS_ROUTES[i % METRICS_ROUTES.len()]
        };
        if let Err(e) = scrape_route(port, route, None) {
            eprintln!("benchmark_meta: metrics request {route} failed: {e}");
            return;
        }
    }
}

/// Both exposition formats: text with classic buckets, and protobuf with native
/// histograms. They share the snapshot and differ in the serializer.
#[cfg(feature = "hotpath-prometheus")]
const PROMETHEUS_ACCEPT: &[&str] = &[
    "text/plain",
    "application/vnd.google.protobuf; proto=io.prometheus.client.MetricFamily; encoding=delimited",
];

#[cfg(feature = "hotpath-prometheus")]
fn scrape_prometheus() {
    let port: u16 = std::env::var("HOTPATH_PROMETHEUS_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6772);

    for i in 0..PROMETHEUS_REQUESTS {
        let accept = PROMETHEUS_ACCEPT[i as usize % PROMETHEUS_ACCEPT.len()];
        if let Err(e) = scrape_route(port, "/metrics", Some(accept)) {
            eprintln!("benchmark_meta: prometheus scrape failed: {e}");
            return;
        }
    }
}

fn scrape_route(port: u16, route: &str, accept: Option<&str>) -> std::io::Result<String> {
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port))?;
    write!(stream, "GET {route} HTTP/1.1\r\nHost: 127.0.0.1\r\n")?;
    if let Some(accept) = accept {
        write!(stream, "Accept: {accept}\r\n")?;
    }
    write!(stream, "Connection: close\r\n\r\n")?;
    stream.flush()?;
    let mut body = Vec::new();
    stream.read_to_end(&mut body)?;
    Ok(String::from_utf8_lossy(&body).into_owned())
}

/// First `"id"` in a metrics response, used to build the log routes above.
fn first_id(body: &str) -> Option<u32> {
    let start = body.find("\"id\":")? + 5;
    let digits = &body[start..];
    let end = digits
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(digits.len());
    digits[..end].parse().ok()
}

fn phase(name: &str, ops: u64, body: impl FnOnce()) {
    let start = Instant::now();
    body();
    report(name, ops, start.elapsed());
}

async fn phase_async(name: &str, ops: u64, body: impl std::future::Future<Output = ()>) {
    let start = Instant::now();
    body.await;
    report(name, ops, start.elapsed());
}

fn report(name: &str, ops: u64, elapsed: std::time::Duration) {
    println!(
        "benchmark_meta: {name} {ops} ops in {elapsed:?} ({:.0} ns/op)",
        elapsed.as_nanos() as f64 / ops as f64
    );
}

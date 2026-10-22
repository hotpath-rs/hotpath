//! Synthetic benchmark for checking hotpath.rs PR comments. It touches every
//! resource type the PR comment policy judges (functions timing and alloc,
//! channels, mutexes, rw_locks, SQL, HTTP, server routes and I/O), and on
//! every run each resource independently draws `fast` or `slow`, where
//! `slow` does 3x the work of `fast` (+200% time or bytes). Two runs therefore
//! differ by +200% or -67% on every resource whose draw changed, so each PR
//! comment shows a mix of regressions and improvements whose expected
//! direction is known.
//!
//! The draw is printed and attached to the report as `user_metadata`
//! (`<resource>=fast|slow`), so a comment can be checked against the two
//! reports it compares. `RANDOM_BENCHMARK_SEED=<u64>` replays a draw.
//!
//! Run with:
//!   cargo run --release -p test-axum --example benchmark_random --features hotpath-alloc

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::extract::State;
use axum::routing::get;
use axum::Router;
use diesel::prelude::*;
use diesel::sql_types::BigInt;
use hotpath::{HotpathGuardBuilder, Section};

/// Calls per resource. Timing rows need at least 10 calls to be judged.
const RUNS: u64 = 40;
/// Time unit of a `fast` operation; `slow` takes `SLOW_FACTOR` times as long.
/// Milliseconds keep the 3x ratio well above timer and CI scheduler noise.
const BASE_DELAY: Duration = Duration::from_millis(2);
const SLOW_FACTOR: u32 = 3;
/// Bytes allocated per `fast` call of the alloc function.
const BASE_ALLOC: usize = 4 * 1024;
/// Rows a `fast` SQL query walks through a recursive CTE (CPU-bound in SQLite).
const BASE_SQL_ROWS: i64 = 20_000;
const CHANNEL_CAPACITY: usize = 4;

#[derive(Clone, Copy, PartialEq)]
enum Speed {
    Fast,
    Slow,
}

impl Speed {
    fn factor(self) -> u32 {
        match self {
            Speed::Fast => 1,
            Speed::Slow => SLOW_FACTOR,
        }
    }

    fn delay(self) -> Duration {
        BASE_DELAY * self.factor()
    }

    fn as_str(self) -> &'static str {
        match self {
            Speed::Fast => "fast",
            Speed::Slow => "slow",
        }
    }
}

#[derive(Clone, Copy)]
enum Resource {
    FunctionsTiming,
    FunctionsAlloc,
    Channels,
    Mutexes,
    RwLocks,
    Sql,
    Http,
    Server,
    Io,
}

const RESOURCES: [Resource; 9] = [
    Resource::FunctionsTiming,
    Resource::FunctionsAlloc,
    Resource::Channels,
    Resource::Mutexes,
    Resource::RwLocks,
    Resource::Sql,
    Resource::Http,
    Resource::Server,
    Resource::Io,
];

impl Resource {
    fn name(self) -> &'static str {
        match self {
            Resource::FunctionsTiming => "functions_timing",
            Resource::FunctionsAlloc => "functions_alloc",
            Resource::Channels => "channels",
            Resource::Mutexes => "mutexes",
            Resource::RwLocks => "rw_locks",
            Resource::Sql => "sql",
            Resource::Http => "http",
            Resource::Server => "server",
            Resource::Io => "io",
        }
    }
}

/// One independent draw per resource, indexed like `RESOURCES`.
struct Plan {
    seed: u64,
    speeds: [Speed; RESOURCES.len()],
}

impl Plan {
    fn draw() -> Self {
        let seed = match std::env::var("RANDOM_BENCHMARK_SEED") {
            Ok(v) => v
                .trim()
                .parse()
                .expect("RANDOM_BENCHMARK_SEED must be a u64"),
            Err(_) => SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock before epoch")
                .as_nanos() as u64,
        };
        let mut state = seed;
        let speeds = std::array::from_fn(|_| {
            if splitmix64(&mut state) & 1 == 0 {
                Speed::Fast
            } else {
                Speed::Slow
            }
        });
        Self { seed, speeds }
    }

    fn speed(&self, resource: Resource) -> Speed {
        self.speeds[resource as usize]
    }

    fn user_metadata(&self) -> HashMap<String, String> {
        let mut metadata: HashMap<String, String> = RESOURCES
            .iter()
            .map(|&r| (r.name().to_string(), self.speed(r).as_str().to_string()))
            .collect();
        metadata.insert("seed".to_string(), self.seed.to_string());
        metadata
    }
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

// `#[hotpath::main]` would install this, but the guard is built by hand here
// to attach the draw as `user_metadata`.
#[cfg(feature = "hotpath-alloc")]
#[global_allocator]
static GLOBAL: hotpath::CountingAllocator = hotpath::CountingAllocator::new();

#[hotpath::measure]
fn random_timing(speed: Speed) {
    std::thread::sleep(speed.delay());
}

#[hotpath::measure]
fn random_alloc(speed: Speed) -> usize {
    let buf = vec![7u8; BASE_ALLOC * speed.factor() as usize];
    std::hint::black_box(&buf);
    buf.len()
}

/// Reader and writer whose every operation takes the resource's delay.
struct DelayedIo {
    delay: Duration,
}

impl Write for DelayedIo {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        std::thread::sleep(self.delay);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Read for DelayedIo {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        std::thread::sleep(self.delay);
        buf.fill(7);
        Ok(buf.len())
    }
}

// Blocks the worker instead of awaiting `tokio::time::sleep`, whose
// millisecond granularity would blur the 3x ratio. Requests arrive one at a
// time, so nothing else waits on that worker.
async fn delayed(State(delay): State<Duration>) -> &'static str {
    std::thread::sleep(delay);
    "ok"
}

/// Serves `router` on a fresh port and returns the URL of its `/work` route.
async fn serve(router: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("local addr").port();
    tokio::spawn(async move {
        axum::serve(listener, router).await.expect("axum server");
    });
    format!("http://127.0.0.1:{port}/work")
}

/// Two threads take turns running `op`, which holds a lock for the resource's
/// delay, so both the hold (acquire) time and the wait time scale with it.
fn contend(runs: u64, op: impl FnMut() + Send + Clone + 'static) {
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let mut op = op.clone();
            std::thread::spawn(move || {
                for _ in 0..runs / 2 {
                    op();
                    // Lets the other thread take the lock before the next turn.
                    std::thread::sleep(Duration::from_micros(200));
                }
            })
        })
        .collect();
    for h in handles {
        h.join().expect("lock worker panicked");
    }
}

#[tokio::main]
async fn main() {
    let plan = Plan::draw();
    println!("benchmark_random: seed {}", plan.seed);
    for &r in &RESOURCES {
        println!("benchmark_random: {} {}", r.name(), plan.speed(r).as_str());
    }

    hotpath::instrument_diesel_sql();
    let _guard = HotpathGuardBuilder::new("main")
        .sections(vec![
            Section::FunctionsTiming,
            Section::FunctionsAlloc,
            Section::Channels,
            Section::Mutexes,
            Section::RwLocks,
            Section::Sql,
            Section::Http,
            Section::Server,
            Section::Io,
            // Not judged by the policy, but feeds the max RSS total.
            Section::Threads,
        ])
        .user_metadata(plan.user_metadata())
        .build();
    let overall = Instant::now();

    let speed = plan.speed(Resource::FunctionsTiming);
    for _ in 0..RUNS {
        random_timing(speed);
    }

    let speed = plan.speed(Resource::FunctionsAlloc);
    let mut total = 0;
    for _ in 0..RUNS {
        total += random_alloc(speed);
    }
    std::hint::black_box(total);

    // The producer fills the bounded channel at once and the consumer takes
    // one message per delay, so every message waits about
    // `CHANNEL_CAPACITY * delay` in the queue.
    let delay = plan.speed(Resource::Channels).delay();
    let (tx, rx) = hotpath::channel!(
        std::sync::mpsc::sync_channel::<u64>(CHANNEL_CAPACITY),
        capacity = CHANNEL_CAPACITY,
        label = "random-channel"
    );
    let producer = std::thread::spawn(move || {
        for i in 0..RUNS {
            tx.send(i).expect("receiver dropped");
        }
    });
    while let Ok(v) = rx.recv() {
        std::thread::sleep(delay);
        std::hint::black_box(v);
    }
    producer.join().expect("producer panicked");

    let hold = plan.speed(Resource::Mutexes).delay();
    let mutex = Arc::new(hotpath::mutex!(
        std::sync::Mutex::new(0u64),
        label = "random-mutex"
    ));
    contend(RUNS, move || {
        let mut v = mutex.lock().expect("poisoned");
        std::thread::sleep(hold);
        *v += 1;
    });

    let hold = plan.speed(Resource::RwLocks).delay();
    let lock = Arc::new(hotpath::rw_lock!(
        std::sync::RwLock::new(0u64),
        label = "random-rw-lock"
    ));
    let mut write = false;
    contend(RUNS, move || {
        write = !write;
        if write {
            let mut w = lock.write().expect("poisoned");
            std::thread::sleep(hold);
            *w += 1;
        } else {
            let r = lock.read().expect("poisoned");
            std::thread::sleep(hold);
            std::hint::black_box(*r);
        }
    });

    let rows = BASE_SQL_ROWS * plan.speed(Resource::Sql).factor() as i64;
    let mut conn = SqliteConnection::establish(":memory:").expect("sqlite");
    for _ in 0..RUNS {
        diesel::sql_query(
            "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < ?) \
             SELECT COUNT(*) FROM c",
        )
        .bind::<BigInt, _>(rows)
        .execute(&mut conn)
        .expect("sql query");
    }

    // Outbound HTTP goes to an uninstrumented server, and the instrumented
    // server is called by a plain client, so the two draws stay independent.
    let http_target = serve(
        Router::new()
            .route("/work", get(delayed))
            .with_state(plan.speed(Resource::Http).delay()),
    )
    .await;
    let server_target = serve(hotpath::axum!(Router::new()
        .route("/work", get(delayed))
        .with_state(plan.speed(Resource::Server).delay())))
    .await;
    let http_client = hotpath::http!(reqwest::Client::new());
    let plain_client = reqwest::Client::new();
    for _ in 0..RUNS {
        http_client
            .get(&http_target)
            .send()
            .await
            .expect("http request");
        plain_client
            .get(&server_target)
            .send()
            .await
            .expect("server request");
    }

    let mut io = hotpath::io!(
        DelayedIo {
            delay: plan.speed(Resource::Io).delay(),
        },
        label = "random-io"
    );
    let mut buf = [0u8; 64];
    for _ in 0..RUNS / 2 {
        io.write_all(&buf).expect("write");
        io.read_exact(&mut buf).expect("read");
    }

    println!("benchmark_random: total {:?}", overall.elapsed());
}

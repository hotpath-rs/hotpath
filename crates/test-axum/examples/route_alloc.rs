//! Demonstrates per-route memory profiling: every request that carries a
//! route scope reports the bytes and allocations made while its handler
//! future was polled - extractors, the handler body, response serialization -
//! and the `server` section prints a second table with bytes per request per
//! route next to the response-time table. `GET /big` builds a 1 MiB body,
//! `GET /small` answers with a static string, and one unmatched request lands
//! in `GET <unmatched>` with no memory attribution (no route scope).
//! `GET /block` measures a block that spans an `.await`, so the block's own
//! allocation frame is still open when the route scope closes for that poll;
//! both halves of the block still count towards the route.
//!
//! Run with:
//!   cargo run -p test-axum --example route_alloc --features hotpath-alloc
//!
//! `NESTED=1` additionally wraps a nested router with its own `AxumLayer`:
//! the inner layer notices the outer scope, stays silent, and the request is
//! reported once by the outer layer (with the nested handler's bytes).
//! `HOTPATH_ROUTE_SCOPE=0` disables the scope, so no memory is attributed.

use axum::routing::get;
use axum::Router;
use std::time::Duration;

const BIG_BYTES: usize = 1024 * 1024;
const BLOCK_PRE_BYTES: usize = 128 * 1024;
const BLOCK_POST_BYTES: usize = 256 * 1024;

// Measured inside the handler: counts toward the route's inclusive total and
// shows up in the `hotpath_function_route_*` families.
#[hotpath::measure]
fn build_body(len: usize) -> Vec<u8> {
    std::hint::black_box(vec![b'x'; len])
}

async fn big() -> Vec<u8> {
    build_body(BIG_BYTES)
}

async fn small() -> &'static str {
    "ok"
}

// `measure_block!` around an `.await`: the block guard outlives the poll that
// suspends, so its allocation frame is still open when the route scope closes.
// The route counts what was allocated before and after the suspension.
async fn block() -> Vec<u8> {
    hotpath::measure_block!("block_await", {
        let pre = std::hint::black_box(vec![b'x'; BLOCK_PRE_BYTES]);
        tokio::task::yield_now().await;
        let post = std::hint::black_box(vec![b'y'; BLOCK_POST_BYTES]);
        drop(pre);
        post
    })
}

#[tokio::main]
#[hotpath::main(report = "server")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let base = format!("http://127.0.0.1:{port}");

    let router = Router::new()
        .route("/big", get(big))
        .route("/small", get(small))
        .route("/block", get(block));
    let app = if std::env::var("NESTED").is_ok_and(|v| v == "1") {
        // A nested router wrapped separately: the inner layer is inert.
        let nested = hotpath::axum!(Router::new().route("/big", get(big)));
        hotpath::axum!(router.nest("/nested", nested))
    } else {
        hotpath::axum!(router)
    };
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("axum server");
    });

    let paths = if std::env::var("NESTED").is_ok_and(|v| v == "1") {
        vec!["/nested/big"; 3]
    } else {
        vec![
            "/big", "/big", "/big", "/small", "/small", "/small", "/small", "/block", "/block",
        ]
    };
    for path in paths {
        let body = ureq::get(format!("{base}{path}"))
            .call()?
            .body_mut()
            .read_to_vec()?;
        std::hint::black_box(body);
    }
    // Unmatched request: no route scope, so no memory attribution.
    let _ = ureq::get(format!("{base}/missing"))
        .config()
        .http_status_as_error(false)
        .build()
        .call()?;

    if let Ok(secs) = std::env::var("TEST_SLEEP_SECONDS") {
        if let Ok(secs) = secs.parse::<u64>() {
            tokio::time::sleep(Duration::from_secs(secs)).await;
        }
    }

    println!("axum route alloc example completed");
    Ok(())
}

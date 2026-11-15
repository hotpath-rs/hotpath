//! Run with:
//!   cargo run -p test-channels-crossbeam --example crossbeam_wrap_little --features hotpath

// Demonstrates Little's Law (L = lambda * W) on top of the crossbeam wrap-channel
// send->receive (dwell time) histogram.
//
// Two unbounded wrap channels are each driven empty -> empty, so the law holds
// exactly over the observation window:
//
//   L  = time-average number of messages waiting in the channel
//   lambda = arrival/throughput rate (messages drained / window seconds)
//   W  = mean send->receive dwell time (the wrap channel's `delay_avg` histogram)
//
//   Case A "keeps up":     consumer faster than producer  -> tiny queue, tiny W
//   Case B "falls behind": consumer slower than producer  -> backlog, large W
//
// For each channel the example measures L by integrating the live queue depth,
// computes lambda from the drained count over the window, and reads W from the
// /channels metrics endpoint (the same histogram printed in the report below).
// lambda * W lands on the observed L in both regimes - the slow consumer trades a
// lower throughput for a proportionally larger dwell time and backlog.
//
// cargo run -p test-channels-crossbeam --example crossbeam_wrap_little --features hotpath
use hotpath::json::JsonChannelsList;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

struct ScenarioResult {
    label: &'static str,
    lambda: f64,     // messages / second drained over the window
    l_observed: f64, // time-average queue depth (integral of depth / window)
}

fn run_scenario(
    label: &'static str,
    tx: hotpath::wrap::crossbeam_channel::Sender<u64>,
    rx: hotpath::wrap::crossbeam_channel::Receiver<u64>,
    producer_interval: Duration,
    service_time: Duration,
    msg_count: u64,
) -> ScenarioResult {
    // Sampler integrates queue depth over time to recover the time-average L.
    let sample_rx = rx.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_sampler = Arc::clone(&stop);
    let sampler = thread::spawn(move || {
        let start = Instant::now();
        let mut last = start;
        let mut area: f64 = 0.0; // sum(depth * dt) in message-seconds
        loop {
            thread::sleep(Duration::from_micros(200));
            let now = Instant::now();
            let dt = now.duration_since(last).as_secs_f64();
            area += sample_rx.len() as f64 * dt;
            last = now;
            if stop_sampler.load(Ordering::Relaxed) {
                break;
            }
        }
        let total = last.duration_since(start).as_secs_f64();
        area / total
    });

    let window_start = Instant::now();

    let producer = thread::spawn(move || {
        for i in 0..msg_count {
            tx.send(i).expect("send failed");
            thread::sleep(producer_interval);
        }
        // tx drops here -> channel disconnects once drained, releasing the consumer.
    });

    let consumer = thread::spawn(move || {
        let mut received = 0u64;
        while let Ok(_msg) = rx.recv() {
            thread::sleep(service_time);
            received += 1;
        }
        received
    });

    producer.join().unwrap();
    let received = consumer.join().unwrap();
    let window = window_start.elapsed();

    stop.store(true, Ordering::Relaxed);
    let l_observed = sampler.join().unwrap();

    let lambda = received as f64 / window.as_secs_f64();
    println!(
        "[{label}] drained {received} msgs in {:.0}ms  ->  lambda = {lambda:.1} msg/s",
        window.as_secs_f64() * 1000.0
    );

    ScenarioResult {
        label,
        lambda,
        l_observed,
    }
}

fn metrics_port() -> u16 {
    std::env::var("HOTPATH_METRICS_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(6770)
}

// Reads each channel's `delay_avg` (mean dwell time W) from the metrics endpoint,
// keyed by label, in nanoseconds. Waits until every channel has fully drained
// (`received_count == expected`) so the histogram includes the late, high-dwell
// messages - crossbeam wrap events are batched per-thread and reach the worker a
// little after the consumer finishes.
fn fetch_dwell_nanos(expected: u64) -> std::collections::HashMap<String, u64> {
    let url = format!("http://localhost:{}/channels", metrics_port());
    let mut out = std::collections::HashMap::new();
    for _ in 0..25 {
        if let Ok(mut resp) = ureq::get(&url).call() {
            if let Ok(body) = resp.body_mut().read_to_string() {
                if let Ok(list) = serde_json::from_str::<JsonChannelsList>(&body) {
                    let all_drained = !list.data.is_empty()
                        && list.data.iter().all(|ch| ch.received_count >= expected);
                    if all_drained {
                        for ch in list.data {
                            if let Some(avg) =
                                ch.delay_avg.as_deref().and_then(hotpath::parse_duration)
                            {
                                out.insert(ch.name, avg);
                            }
                        }
                        return out;
                    }
                }
            }
        }
        thread::sleep(Duration::from_millis(200));
    }
    out
}

fn main() {
    let guard = hotpath::HotpathGuardBuilder::new("main")
        .sections(vec![hotpath::Section::Channels])
        .percentiles(&[50.0, 95.0, 99.0])
        .build();

    println!("Little's Law demo (L = lambda * W) on crossbeam wrap channels:\n");

    // Distinct `channel!` call sites so each keeps its own label (a shared source
    // line would get a `-N` disambiguation suffix).
    let (a_tx, a_rx) = hotpath::channel!(crossbeam_channel::unbounded::<u64>(), label = "keeps-up");
    let (b_tx, b_rx) = hotpath::channel!(
        crossbeam_channel::unbounded::<u64>(),
        label = "falls-behind"
    );

    // Producer fires every 1ms (~1000 msg/s arrival). The consumer's service time
    // decides whether it keeps up.
    let keeps_up = run_scenario(
        "keeps-up",
        a_tx,
        a_rx,
        Duration::from_millis(1),
        Duration::from_micros(400),
        200,
    );
    let falls_behind = run_scenario(
        "falls-behind",
        b_tx,
        b_rx,
        Duration::from_millis(1),
        Duration::from_millis(3),
        200,
    );

    let dwell = fetch_dwell_nanos(200);

    println!("\nLittle's Law check (L = lambda * W):\n");
    println!(
        "  {:<14} {:>11} {:>12} {:>14} {:>14}",
        "channel", "lambda(/s)", "W (dwell)", "lambda*W (L)", "L observed"
    );
    for r in [&keeps_up, &falls_behind] {
        let w_ns = dwell.get(r.label).copied().unwrap_or(0);
        let w_secs = w_ns as f64 / 1_000_000_000.0;
        let l_predicted = r.lambda * w_secs;
        println!(
            "  {:<14} {:>11.1} {:>12} {:>14.2} {:>14.2}",
            r.label,
            r.lambda,
            hotpath::format_duration(w_ns),
            l_predicted,
            r.l_observed,
        );
    }
    println!(
        "\n  lambda*W tracks the observed average queue depth in both regimes - the\n  slow consumer's larger backlog is exactly its larger dwell time W.\n"
    );

    // Report below prints the per-channel dwell-time histogram (Delay avg / p50 / p95 / p99).
    drop(guard);

    println!("\nExample completed!");
}

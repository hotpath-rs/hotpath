//! Sample use of hotpath-drain: several producer threads push one million
//! events in total into a shared registry while a single consumer thread
//! sweeps the queues, then drains what is left at shutdown.
//!
//! Producers and consumer run in lockstep rounds, so sweep counts and
//! allocations are fixed by the constants below rather than by scheduling.
//! Changing the constants breaks comparability with previously uploaded reports.
//!
//! Run with:
//!   cargo run -p hotpath-drain --example throughput --release
//!   cargo run -p hotpath-drain --example throughput --release --features hotpath-meta
//!   cargo run -p hotpath-drain --example throughput --release --features hotpath-meta,hotpath-alloc-meta
//!   CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER="valgrind -q --tool=callgrind" cargo run -p hotpath-drain --example throughput --release --features hotpath-meta,hotpath-alloc-meta,hotpath-callgrind-meta

use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Instant;

use hotpath_drain::{EventProducer, EventQueueRegistry};

const PRODUCERS: usize = 4;
const ROUNDS: u64 = 20;
/// Events each producer pushes per round; stays below the per-sweep chunk cap
/// so one sweep always drains a whole round.
const BATCH: u64 = 12_500;
const EVENTS_PER_PRODUCER: u64 = ROUNDS * BATCH;
const TOTAL_EVENTS: u64 = PRODUCERS as u64 * EVENTS_PER_PRODUCER;

/// One event, stored inline: it owns no heap memory, so the only allocations
/// left under profiling are the queue's own chunk storage.
struct Event {
    producer: usize,
    seq: u64,
    payload: [u8; 8],
}

// The policy's `push` budget is sized for 24 B slots.
const _: () = assert!(size_of::<Event>() == 24);

static REGISTRY: EventQueueRegistry<Event> = EventQueueRegistry::new();

thread_local! {
    static PRODUCER: EventProducer<Event> = REGISTRY.register();
}

/// Producers and consumer meet here twice per round: once when every batch
/// is pushed, once when the sweep is done.
struct Lockstep {
    batch_pushed: Barrier,
    sweep_done: Barrier,
}

impl Lockstep {
    fn new() -> Self {
        Self {
            batch_pushed: Barrier::new(PRODUCERS + 1),
            sweep_done: Barrier::new(PRODUCERS + 1),
        }
    }
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure)]
fn produce(producer: usize, lockstep: &Lockstep) {
    for round in 0..ROUNDS {
        for i in 0..BATCH {
            let event = Event {
                producer,
                seq: round * BATCH + i,
                payload: [i as u8; 8],
            };
            let _ = PRODUCER.try_with(|p| p.push(event));
        }
        lockstep.batch_pushed.wait();
        lockstep.sweep_done.wait();
    }
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure)]
fn consume(batch: &[Event]) -> u64 {
    let mut checksum = 0u64;
    for event in batch {
        checksum = checksum
            .wrapping_add(event.producer as u64)
            .wrapping_add(event.seq)
            .wrapping_add(event.payload[0] as u64);
    }
    checksum
}

/// One sweep per round, then a final drain once the producers have exited
/// (signalled through `producers_exited`) so no event is lost. Returns the
/// total consumed.
#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure)]
fn consumer_loop(lockstep: Arc<Lockstep>, producers_exited: Arc<Barrier>) -> u64 {
    // Sized for one round, the most a sweep can return since the vector is
    // cleared after each one, so draining never reallocates.
    let mut batch = Vec::with_capacity((PRODUCERS as u64 * BATCH) as usize);
    let mut consumed = 0u64;
    let mut checksum = 0u64;
    for _ in 0..ROUNDS {
        lockstep.batch_pushed.wait();
        REGISTRY.sweep(&mut batch);
        consumed += batch.len() as u64;
        checksum = checksum.wrapping_add(consume(&batch));
        batch.clear();
        lockstep.sweep_done.wait();
    }
    producers_exited.wait();
    REGISTRY.drain_all(&mut batch);
    consumed += batch.len() as u64;
    checksum = checksum.wrapping_add(consume(&batch));
    std::hint::black_box(checksum);
    consumed
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::main(percentiles = [95, 99, 99.9]))]
fn main() {
    REGISTRY.set_active(true);
    let start = Instant::now();
    let lockstep = Arc::new(Lockstep::new());
    let producers_exited = Arc::new(Barrier::new(2));

    let consumer = thread::Builder::new()
        .name("consumer".into())
        .spawn({
            let lockstep = Arc::clone(&lockstep);
            let producers_exited = Arc::clone(&producers_exited);
            move || consumer_loop(lockstep, producers_exited)
        })
        .expect("spawn consumer");

    let producers: Vec<_> = (0..PRODUCERS)
        .map(|id| {
            let lockstep = Arc::clone(&lockstep);
            thread::Builder::new()
                .name(format!("producer-{id}"))
                .spawn(move || produce(id, &lockstep))
                .expect("spawn producer")
        })
        .collect();
    for handle in producers {
        handle.join().expect("producer panicked");
    }

    // Producers have exited, so their thread-local `EventProducer`s are
    // dropped and the queues are marked closed before the final drain.
    REGISTRY.set_active(false);
    producers_exited.wait();
    let consumed = consumer.join().expect("consumer panicked");
    let elapsed = start.elapsed();

    assert_eq!(consumed, TOTAL_EVENTS, "events lost in transit");
    println!(
        "{PRODUCERS} producers sent {TOTAL_EVENTS} events, consumer received {consumed} in {elapsed:.2?} ({:.1} M events/s)",
        TOTAL_EVENTS as f64 / elapsed.as_secs_f64() / 1e6
    );
}

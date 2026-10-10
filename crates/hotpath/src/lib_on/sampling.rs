//! Time-sampling support: measure durations for only a fraction of calls while
//! keeping every event flowing (counts, states, queue sizes stay exact).
//!
//! Rates are fractions in `[0.0, 1.0]`: `0.1` times about 1 in 10 calls, `0.0`
//! is count-only mode (no durations at all), `1.0` or unset measures everything.
//! Every decision is an independent random draw from a per-thread generator,
//! so periodic workloads cannot lock onto a fixed phase.
//! Resolution happens once at guard build; events emitted before the guard
//! exists are measured at 100%.

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Sampler {
    /// Count-only mode (rate 0): never measure durations.
    Never,
    /// Time each call independently with probability `rate`; `threshold` is
    /// the rate scaled to the `u64` range.
    Fraction { rate: f64, threshold: u64 },
}

impl Sampler {
    /// `None` means "measure everything" (rate `1.0`, or an invalid rate).
    pub(crate) fn from_rate(rate: f64) -> Option<Self> {
        if !rate.is_finite() || !(0.0..=1.0).contains(&rate) {
            return None;
        }
        if rate == 0.0 {
            return Some(Sampler::Never);
        }
        if rate >= 1.0 {
            return None;
        }
        // `u64::MAX as f64` rounds up to 2^64, so `threshold / 2^64 == rate`.
        let threshold = (rate * u64::MAX as f64) as u64;
        Some(Sampler::Fraction { rate, threshold })
    }

    /// Fraction of calls timed (0.0 in count-only mode).
    pub(crate) fn effective_rate(&self) -> f64 {
        match self {
            Sampler::Never => 0.0,
            Sampler::Fraction { rate, .. } => *rate,
        }
    }

    /// One independent random decision.
    #[inline]
    fn sample(&self, rng: &Cell<u64>) -> bool {
        match self {
            Sampler::Never => false,
            Sampler::Fraction { threshold, .. } => next_u64(rng) < *threshold,
        }
    }
}

/// wyrand step: one add and one 64x64 -> 128 multiply, any seed is valid.
#[inline]
fn next_u64(state: &Cell<u64>) -> u64 {
    let s = state.get().wrapping_add(0xa076_1d64_78bd_642f);
    state.set(s);
    let t = u128::from(s) * u128::from(s ^ 0xe703_7ed1_a0b4_28db);
    (t as u64) ^ ((t >> 64) as u64)
}

/// Per-thread seed: the counter keeps threads apart, the wall clock varies
/// runs. No thread-local or OS entropy access, so it is safe during thread
/// teardown.
fn seed() -> u64 {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    nanos ^ seq.wrapping_mul(0x9e37_79b9_7f4a_7c15)
}

/// Per-resource sampling slot, set once at guard build. Unset means measure
/// every call, so pre-guard events keep exact timings.
pub(crate) struct ResourceSampling {
    slot: OnceLock<Option<Sampler>>,
}

impl ResourceSampling {
    const fn new() -> Self {
        Self {
            slot: OnceLock::new(),
        }
    }

    fn set(&self, sampler: Option<Sampler>) {
        let _ = self.slot.set(sampler);
    }

    #[inline]
    pub(crate) fn sampler(&self) -> Option<Sampler> {
        self.slot.get().copied().flatten()
    }
}

pub(crate) static FUNCTIONS_SAMPLING: ResourceSampling = ResourceSampling::new();
pub(crate) static MUTEXES_SAMPLING: ResourceSampling = ResourceSampling::new();
pub(crate) static RW_LOCKS_SAMPLING: ResourceSampling = ResourceSampling::new();
pub(crate) static FUTURES_SAMPLING: ResourceSampling = ResourceSampling::new();
pub(crate) static CHANNELS_SAMPLING: ResourceSampling = ResourceSampling::new();
pub(crate) static IO_SAMPLING: ResourceSampling = ResourceSampling::new();

thread_local! {
    /// Generator state shared by every resource on the thread.
    static RNG: Cell<u64> = Cell::new(seed());
}

/// Independent per-call decision. `try_with` because guards can drop during
/// thread teardown when the thread-local is already destroyed. A failed
/// attempt (try-lock miss, retryable I/O error) needs no rollback since the
/// next draw is independent.
#[inline]
fn should_time(sampling: &ResourceSampling) -> bool {
    match sampling.sampler() {
        None => true,
        // Count-only mode never times; skip the generator access entirely.
        Some(Sampler::Never) => false,
        Some(sampler) => RNG.try_with(|rng| sampler.sample(rng)).unwrap_or(false),
    }
}

#[inline]
pub(crate) fn functions_should_time() -> bool {
    should_time(&FUNCTIONS_SAMPLING)
}

#[inline]
pub(crate) fn mutexes_should_time() -> bool {
    should_time(&MUTEXES_SAMPLING)
}

#[inline]
pub(crate) fn rw_locks_should_time() -> bool {
    should_time(&RW_LOCKS_SAMPLING)
}

#[inline]
pub(crate) fn futures_should_time() -> bool {
    should_time(&FUTURES_SAMPLING)
}

#[inline]
pub(crate) fn io_should_time() -> bool {
    should_time(&IO_SAMPLING)
}

/// Wrap-channel decision, made once on the send side; the resulting stamp
/// travels in the payload so the receiver needs no decision of its own.
#[inline]
pub(crate) fn channels_should_time() -> bool {
    should_time(&CHANNELS_SAMPLING)
}

/// Builder-provided rates, resolved against env vars at guard build.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct TimeSamplingConfig {
    pub(crate) global: Option<f64>,
    pub(crate) functions: Option<f64>,
    pub(crate) mutexes: Option<f64>,
    pub(crate) rw_locks: Option<f64>,
    pub(crate) futures: Option<f64>,
    pub(crate) channels: Option<f64>,
    pub(crate) io: Option<f64>,
}

/// Invalid values (negative, > 1.0, NaN, unparseable) are ignored, falling
/// through to the next precedence level.
fn parse_rate_env(name: &str) -> Option<f64> {
    let rate: f64 = std::env::var(name).ok()?.parse().ok()?;
    (rate.is_finite() && (0.0..=1.0).contains(&rate)).then_some(rate)
}

/// Resolves and locks in per-resource samplers. Precedence:
/// per-resource env > global env > per-resource builder > global builder.
pub(crate) fn init_time_sampling_rate(config: &TimeSamplingConfig) {
    let global_env = parse_rate_env("HOTPATH_TIME_SAMPLING_RATE");
    let resolve = |env_name: &str, builder_rate: Option<f64>| {
        parse_rate_env(env_name)
            .or(global_env)
            .or(builder_rate)
            .or(config.global)
            .and_then(Sampler::from_rate)
    };
    let functions = resolve("HOTPATH_FUNCTIONS_TIME_SAMPLING_RATE", config.functions);
    // An untimed child has no duration to subtract, so its time would land in
    // its parent's exclusive time.
    if *crate::lib_on::functions::exclusive::TIME_EXCLUSIVE && functions.is_some() {
        eprintln!(
            "[hotpath] HOTPATH_TIME_EXCLUSIVE needs every function call timed, \
             ignoring the functions time sampling rate"
        );
        FUNCTIONS_SAMPLING.set(None);
    } else {
        FUNCTIONS_SAMPLING.set(functions);
    }
    MUTEXES_SAMPLING.set(resolve(
        "HOTPATH_MUTEXES_TIME_SAMPLING_RATE",
        config.mutexes,
    ));
    RW_LOCKS_SAMPLING.set(resolve(
        "HOTPATH_RW_LOCKS_TIME_SAMPLING_RATE",
        config.rw_locks,
    ));
    FUTURES_SAMPLING.set(resolve(
        "HOTPATH_FUTURES_TIME_SAMPLING_RATE",
        config.futures,
    ));
    CHANNELS_SAMPLING.set(resolve(
        "HOTPATH_CHANNELS_TIME_SAMPLING_RATE",
        config.channels,
    ));
    IO_SAMPLING.set(resolve("HOTPATH_IO_TIME_SAMPLING_RATE", config.io));
}

/// Effective per-resource rates for the report header / JSON, `None` when no
/// sampler is active.
pub(crate) fn active_rates() -> Option<HashMap<String, f64>> {
    let mut rates = HashMap::new();
    for (name, sampling) in [
        ("functions", &FUNCTIONS_SAMPLING),
        ("mutexes", &MUTEXES_SAMPLING),
        ("rw_locks", &RW_LOCKS_SAMPLING),
        ("futures", &FUTURES_SAMPLING),
        ("channels", &CHANNELS_SAMPLING),
        ("io", &IO_SAMPLING),
    ] {
        if let Some(sampler) = sampling.sampler() {
            rates.insert(name.to_string(), sampler.effective_rate());
        }
    }
    (!rates.is_empty()).then_some(rates)
}

#[cfg(test)]
mod tests {
    use crate::lib_on::sampling::*;

    fn fraction(rate: f64) -> Sampler {
        match Sampler::from_rate(rate) {
            Some(s @ Sampler::Fraction { .. }) => s,
            other => panic!("expected a Fraction sampler for {rate}, got {other:?}"),
        }
    }

    #[test]
    fn from_rate_bounds() {
        assert_eq!(Sampler::from_rate(1.0), None);
        assert_eq!(Sampler::from_rate(0.0), Some(Sampler::Never));
        assert_eq!(Sampler::from_rate(-0.1), None);
        assert_eq!(Sampler::from_rate(1.5), None);
        assert_eq!(Sampler::from_rate(f64::NAN), None);
        for rate in [0.001, 0.1, 0.3, 0.5, 0.9] {
            assert_eq!(fraction(rate).effective_rate(), rate);
        }
        assert_eq!(
            fraction(0.5),
            Sampler::Fraction {
                rate: 0.5,
                threshold: 1 << 63
            }
        );
    }

    /// Standard deviation is at most 158 for n = 100_000, so 1_500 is over
    /// nine sigmas at every rate tested.
    #[test]
    fn fraction_keeps_rate_on_average() {
        let n = 100_000u64;
        for rate in [0.01, 0.1, 0.5, 0.9] {
            let s = fraction(rate);
            let rng = Cell::new(seed());
            let kept = (0..n).filter(|_| s.sample(&rng)).count() as f64;
            let expected = rate * n as f64;
            assert!(
                (kept - expected).abs() < 1_500.0,
                "rate {rate}: kept {kept}, expected {expected}"
            );
        }
        let rng = Cell::new(seed());
        assert!(!(0..n).any(|_| Sampler::Never.sample(&rng)));
    }

    /// Two interleaved streams must both receive a fair share of the
    /// decisions (a 1-in-2 counter would sample only the first).
    #[test]
    fn interleaved_streams_are_both_sampled() {
        let s = fraction(0.5);
        let rng = Cell::new(seed());
        let mut kept = [0u32; 2];
        for i in 0..10_000 {
            if s.sample(&rng) {
                kept[i % 2] += 1;
            }
        }
        for (stream, kept) in kept.iter().enumerate() {
            assert!(
                (2_000..=3_000).contains(kept),
                "stream {stream} kept {kept} of 5_000"
            );
        }
    }

    #[test]
    fn threads_draw_different_sequences() {
        let a = Cell::new(seed());
        let b = Cell::new(seed());
        let sa: Vec<u64> = (0..8).map(|_| next_u64(&a)).collect();
        let sb: Vec<u64> = (0..8).map(|_| next_u64(&b)).collect();
        assert_ne!(sa, sb);
    }
}

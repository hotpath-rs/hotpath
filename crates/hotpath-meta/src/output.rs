use serde::ser::Serializer;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::LazyLock;

const DEFAULT_MAX_LOG_LEN: usize = 1536;
pub static MAX_LOG_LEN: LazyLock<usize> = LazyLock::new(|| {
    std::env::var("HOTPATH_META_MAX_LOG_LEN")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_MAX_LOG_LEN)
});

const DEFAULT_FUNCTIONS_NAME_DEPTH: usize = 2;
pub(crate) static FUNCTIONS_NAME_DEPTH: LazyLock<usize> = LazyLock::new(|| {
    std::env::var("HOTPATH_META_FUNCTIONS_NAME_DEPTH")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_FUNCTIONS_NAME_DEPTH)
});

/// Formats a duration in nanoseconds into a human-readable string with appropriate units.
pub fn format_duration(ns: u64) -> String {
    if ns < 1_000 {
        format!("{} ns", ns)
    } else if ns < 1_000_000 {
        format!("{:.2} µs", ns as f64 / 1_000.0)
    } else if ns < 1_000_000_000 {
        format!("{:.2} ms", ns as f64 / 1_000_000.0)
    } else {
        format!("{:.2} s", ns as f64 / 1_000_000_000.0)
    }
}

/// Lossless variant of [`format_duration`]: the same unit bands, with enough
/// decimals to carry every nanosecond (3 for µs, 6 for ms, 9 for s), so
/// [`parse_duration`] recovers the exact value. Used for cloud reports, where
/// the server diffs the numbers instead of a person reading them.
#[cfg(feature = "hotpath-meta")]
pub(crate) fn format_duration_exact(ns: u64) -> String {
    if ns < 1_000 {
        format!("{} ns", ns)
    } else if ns < 1_000_000 {
        format!("{:.3} µs", ns as f64 / 1_000.0)
    } else if ns < 1_000_000_000 {
        format!("{:.6} ms", ns as f64 / 1_000_000.0)
    } else {
        format!("{:.9} s", ns as f64 / 1_000_000_000.0)
    }
}

/// A non-negative, finite number scaled to an integer, rounded.
/// A float-to-int cast saturates and maps NaN to 0, so a value that is
/// not a magnitude is rejected before the cast rather than silently
/// becoming 0 or `u64::MAX`. The upper bound is strict because
/// `u64::MAX as f64` rounds up to 2^64, which would saturate.
fn scaled(num: &str, scale: f64) -> Option<u64> {
    let v: f64 = num.trim().parse().ok()?;
    let scaled = v * scale;
    (v >= 0.0 && scaled.is_finite() && scaled < u64::MAX as f64).then(|| scaled.round() as u64)
}

/// Parses a human-readable duration string back to nanoseconds.
/// Inverse of [`format_duration`]; also reads the lossless form cloud
/// reports carry (`1.004999 ms`) and the ASCII `us` spelling of `µs`.
/// Negative, non-finite and out-of-range values are `None`.
pub fn parse_duration(s: &str) -> Option<u64> {
    let s = s.trim();
    if let Some(num) = s.strip_suffix(" ns") {
        scaled(num, 1.0)
    } else if let Some(num) = s.strip_suffix(" µs").or_else(|| s.strip_suffix(" us")) {
        scaled(num, 1_000.0)
    } else if let Some(num) = s.strip_suffix(" ms") {
        scaled(num, 1_000_000.0)
    } else if let Some(num) = s.strip_suffix(" s") {
        scaled(num, 1_000_000_000.0)
    } else {
        None
    }
}

/// Formats a percentile value for use as a map key (e.g., `"p95"`, `"p99.9"`).
pub fn format_percentile_key(p: f64) -> String {
    if p.fract() == 0.0 {
        format!("p{}", p as u64)
    } else {
        format!("p{}", p)
    }
}

/// Formats a percentile value for display as a column header (e.g., `"P95"`, `"P99.9"`).
pub fn format_percentile_header(p: f64) -> String {
    if p.fract() == 0.0 {
        format!("P{}", p as u64)
    } else {
        format!("P{}", p)
    }
}

/// Formats a byte count into a human-readable string (e.g., "1.5 MB").
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    const THRESHOLD: f64 = 1024.0;

    if bytes == 0 {
        return "0 B".to_string();
    }

    let bytes_f = bytes as f64;
    let unit_index = (bytes_f.log(THRESHOLD).floor() as usize).min(UNITS.len() - 1);
    let unit_value = bytes_f / THRESHOLD.powi(unit_index as i32);

    if unit_index == 0 {
        format!("{} {}", bytes, UNITS[unit_index])
    } else {
        format!("{:.1} {}", unit_value, UNITS[unit_index])
    }
}

/// Lossless variant of [`format_bytes`]: the plain byte count with the `B`
/// unit, so [`parse_bytes`] recovers the exact value. A `KB` figure with one
/// decimal is a 10% step just above 1 KB, which is coarser than the changes a
/// cloud report is diffed for.
#[cfg(feature = "hotpath-meta")]
pub(crate) fn format_bytes_exact(bytes: u64) -> String {
    format!("{} B", bytes)
}

/// How a report renders durations and byte counts. `Display` rounds for
/// reading; `Exact` keeps every nanosecond and byte so a consumer can parse
/// the value back without loss. Cloud reports (`HOTPATH_UPLOAD=1` or JSON
/// output with `hotpath-cloud`) use `Exact`, everything else `Display`.
#[cfg(feature = "hotpath-meta")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Precision {
    Display,
    Exact,
}

#[cfg(feature = "hotpath-meta")]
impl Precision {
    pub(crate) fn for_cloud(cloud: bool) -> Self {
        if cloud {
            Precision::Exact
        } else {
            Precision::Display
        }
    }

    pub(crate) fn duration(self, ns: u64) -> String {
        match self {
            Precision::Display => format_duration(ns),
            Precision::Exact => format_duration_exact(ns),
        }
    }

    pub(crate) fn bytes(self, bytes: u64) -> String {
        match self {
            Precision::Display => format_bytes(bytes),
            Precision::Exact => format_bytes_exact(bytes),
        }
    }

    pub(crate) fn bytes_signed(self, bytes: i64) -> String {
        let sign = if bytes < 0 { "-" } else { "" };
        format!("{sign}{}", self.bytes(bytes.unsigned_abs()))
    }
}

/// Formats an optional per-second rate to one decimal place, or `-` when absent.
pub fn format_rate(rate: Option<f64>) -> String {
    rate.map_or_else(|| "-".to_string(), |v| format!("{v:.1}"))
}

/// Parses a human-readable byte string back to a byte count.
/// Inverse of [`format_bytes`]; also reads the plain `N B` form cloud
/// reports carry. Negative, non-finite and out-of-range values are `None`.
pub fn parse_bytes(s: &str) -> Option<u64> {
    let s = s.trim();
    if let Some(num) = s.strip_suffix(" TB") {
        scaled(num, 1024.0_f64.powi(4))
    } else if let Some(num) = s.strip_suffix(" GB") {
        scaled(num, 1024.0_f64.powi(3))
    } else if let Some(num) = s.strip_suffix(" MB") {
        scaled(num, 1024.0_f64.powi(2))
    } else if let Some(num) = s.strip_suffix(" KB") {
        scaled(num, 1024.0)
    } else if let Some(num) = s.strip_suffix(" B") {
        num.trim().parse::<u64>().ok()
    } else {
        None
    }
}

/// Formats an allocation count as a string.
pub fn format_count(count: u64) -> String {
    count.to_string()
}

/// Parses a count string back to a u64.
/// Inverse of [`format_count`].
pub fn parse_count(s: &str) -> Option<u64> {
    s.trim().parse::<u64>().ok()
}

/// Profiling mode indicating what type of measurements were collected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfilingMode {
    /// Time-based profiling (execution duration)
    Timing,
    /// Allocation profiling with bytes as primary metric
    AllocBytes,
    /// Allocation profiling with count as primary metric
    AllocCount,
    /// Inclusive instructions executed, counted by Valgrind Callgrind
    Icount,
}

impl fmt::Display for ProfilingMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProfilingMode::Timing => write!(f, "timing"),
            ProfilingMode::AllocBytes => write!(f, "alloc-bytes"),
            ProfilingMode::AllocCount => write!(f, "alloc-count"),
            ProfilingMode::Icount => write!(f, "icount"),
        }
    }
}

impl Serialize for ProfilingMode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            ProfilingMode::Timing => serializer.serialize_str("timing"),
            ProfilingMode::AllocBytes => serializer.serialize_str("alloc-bytes"),
            ProfilingMode::AllocCount => serializer.serialize_str("alloc-count"),
            ProfilingMode::Icount => serializer.serialize_str("icount"),
        }
    }
}

impl<'de> Deserialize<'de> for ProfilingMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "timing" => Ok(ProfilingMode::Timing),
            "alloc-bytes" => Ok(ProfilingMode::AllocBytes),
            "alloc-count" => Ok(ProfilingMode::AllocCount),
            "icount" => Ok(ProfilingMode::Icount),
            _ => Err(serde::de::Error::unknown_variant(
                &s,
                &["timing", "alloc-bytes", "alloc-count", "icount"],
            )),
        }
    }
}

pub fn floor_char_boundary(s: &str, index: usize) -> usize {
    if index >= s.len() {
        return s.len();
    }
    let mut i = index;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

pub fn ceil_char_boundary(s: &str, index: usize) -> usize {
    if index >= s.len() {
        return s.len();
    }
    let mut i = index;
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

pub fn shorten_function_name(function_name: &str) -> String {
    let depth = *FUNCTIONS_NAME_DEPTH;
    if depth == 0 {
        return function_name.to_string();
    }
    let parts: Vec<&str> = function_name.split("::").collect();
    if parts.len() > depth {
        parts[parts.len() - depth..].join("::")
    } else {
        function_name.to_string()
    }
}

/// A single log entry for a function invocation. In alloc mode `value` and
/// `alloc_count` are None when the data is invalid (cross-thread or
/// unsupported async).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub(crate) struct FunctionLog {
    /// Measured value (duration in ns for timing, bytes for memory). None if invalid.
    pub value: Option<u64>,
    /// Timestamp when the measurement was taken (nanoseconds since profiler start)
    pub elapsed_nanos: u64,
    /// Allocation count (only for memory mode)
    pub alloc_count: Option<u64>,
    /// Thread ID where the function was executed, None if cross-thread execution
    pub tid: Option<u64>,
    /// Debug representation of the return value (when log = true)
    pub result: Option<String>,
}

/// Response containing recent logs for a function
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct FunctionLogsList {
    pub function_name: String,
    pub logs: Vec<FunctionLog>,
    /// Total number of times this function was invoked (used to calculate invocation numbers)
    pub count: usize,
}

#[cfg(test)]
mod parse_tests {
    use crate::output::*;

    #[test]
    fn test_parse_duration_units() {
        assert_eq!(parse_duration("123 ns"), Some(123));
        assert_eq!(parse_duration("0 ns"), Some(0));
        assert_eq!(parse_duration("1.23 µs"), Some(1230));
        assert_eq!(parse_duration("1.23 ms"), Some(1230000));
        assert_eq!(parse_duration("1.23 s"), Some(1230000000));
        assert_eq!(parse_duration("-0 ns"), Some(0));
        assert_eq!(parse_duration("1e3 ns"), Some(1000));
        assert_eq!(parse_duration("250 us"), parse_duration("250 µs"));
    }

    #[test]
    fn test_parse_duration_invalid() {
        assert_eq!(parse_duration(""), None);
        assert_eq!(parse_duration("invalid"), None);
        assert_eq!(parse_duration("abc ns"), None);
        assert_eq!(parse_duration("-5 ms"), None);
        assert_eq!(parse_duration("inf ms"), None);
        assert_eq!(parse_duration("-inf ns"), None);
        assert_eq!(parse_duration("NaN s"), None);
        assert_eq!(parse_duration("1e400 ms"), None);
        assert_eq!(parse_duration("1e300 s"), None);
        assert_eq!(parse_duration("2e19 ns"), None);
    }

    #[test]
    fn test_parse_duration_roundtrip() {
        for val in [0, 1, 500, 999, 1000, 50_000, 1_230_000, 1_230_000_000] {
            let formatted = format_duration(val);
            let parsed = parse_duration(&formatted);
            assert_eq!(
                parsed,
                Some(val),
                "round-trip failed for {val}: formatted as '{formatted}'"
            );
        }
    }

    #[test]
    fn test_parse_bytes_units() {
        assert_eq!(parse_bytes("0 B"), Some(0));
        assert_eq!(parse_bytes("123 B"), Some(123));
        assert_eq!(parse_bytes("1.5 KB"), Some(1536));
        assert_eq!(parse_bytes("1.0 MB"), Some(1048576));
        assert_eq!(parse_bytes("1.0 GB"), Some(1073741824));
        assert_eq!(parse_bytes("0.5 TB"), Some(549755813888));
        assert_eq!(parse_bytes("0.0 KB"), Some(0));
    }

    #[test]
    fn test_parse_bytes_invalid() {
        assert_eq!(parse_bytes(""), None);
        assert_eq!(parse_bytes("invalid"), None);
        assert_eq!(parse_bytes("abc KB"), None);
        assert_eq!(parse_bytes("-1.5 KB"), None);
        assert_eq!(parse_bytes("-1 B"), None);
        assert_eq!(parse_bytes("NaN KB"), None);
        assert_eq!(parse_bytes("inf MB"), None);
        assert_eq!(parse_bytes("1e300 TB"), None);
    }

    #[test]
    fn test_parse_bytes_roundtrip() {
        for val in [0, 100, 1023, 1024, 1536, 1048576, 1073741824] {
            let formatted = format_bytes(val);
            let parsed = parse_bytes(&formatted);
            assert_eq!(
                parsed,
                Some(val),
                "round-trip failed for {val}: formatted as '{formatted}'"
            );
        }
    }

    #[test]
    fn test_format_count() {
        assert_eq!(format_count(0), "0");
        assert_eq!(format_count(999), "999");
        assert_eq!(format_count(1_000), "1000");
        assert_eq!(format_count(1_000_000), "1000000");
    }

    #[test]
    fn test_parse_count_roundtrip() {
        for val in [0, 1, 500, 999, 1_000, 1_500, 50_000, 1_000_000] {
            let formatted = format_count(val);
            let parsed = parse_count(&formatted);
            assert_eq!(
                parsed,
                Some(val),
                "round-trip failed for {val}: formatted as '{formatted}'"
            );
        }
    }
}

#[cfg(all(test, feature = "hotpath-meta"))]
mod precision_tests {
    use crate::output::*;

    #[test]
    fn test_format_duration_exact_roundtrip() {
        // Band edges, values the display format rounds away, and the
        // 1000 s histogram ceiling.
        for val in [
            0,
            1,
            999,
            1_000,
            1_001,
            999_999,
            1_000_000,
            1_004_999,
            1_234_567,
            999_999_999,
            1_000_000_000,
            1_000_000_001,
            123_456_789_012,
            1_000_000_000_000,
        ] {
            let formatted = format_duration_exact(val);
            assert_eq!(
                parse_duration(&formatted),
                Some(val),
                "round-trip failed for {val}: formatted as '{formatted}'"
            );
        }
        assert_eq!(format_duration_exact(999), "999 ns");
        assert_eq!(format_duration_exact(1_001), "1.001 µs");
        assert_eq!(format_duration_exact(1_004_999), "1.004999 ms");
        assert_eq!(format_duration_exact(1_000_000_001), "1.000000001 s");
    }

    #[test]
    fn test_precision_selects_formatter() {
        assert_eq!(Precision::for_cloud(false), Precision::Display);
        assert_eq!(Precision::for_cloud(true), Precision::Exact);
        assert_eq!(Precision::Display.duration(1_004_999), "1.00 ms");
        assert_eq!(Precision::Exact.duration(1_004_999), "1.004999 ms");
        assert_eq!(Precision::Display.bytes(1_075), "1.0 KB");
        assert_eq!(Precision::Exact.bytes(1_075), "1075 B");
    }

    #[test]
    fn test_format_bytes_exact_roundtrip() {
        for val in [0, 1, 1_023, 1_024, 1_075, 65_229, 1_048_576, u64::MAX] {
            let formatted = format_bytes_exact(val);
            assert_eq!(
                parse_bytes(&formatted),
                Some(val),
                "round-trip failed for {val}: formatted as '{formatted}'"
            );
        }
    }
}

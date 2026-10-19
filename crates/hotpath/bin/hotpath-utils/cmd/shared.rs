use hotpath::json::{
    format_bytes_signed, parse_bytes_signed, JsonFunctionEntry, JsonFunctionsList, JsonReport,
    JsonThreadEntry, JsonThreadsList,
};
use hotpath::table::{Cell, Table};
use hotpath::{
    format_bytes, format_count, parse_bytes, parse_count, parse_duration, shorten_function_name,
};
use std::fmt;
use std::time::Duration;

pub const DEPRECATION_NOTE: &str = "hotpath-utils is deprecated and will be removed in a future release. Use hotpath cloud instead: https://hotpath.rs/cloud";

#[derive(Debug, Clone)]
pub enum MetricDiff {
    CallsCount(u64, u64),  // (before, after)
    DurationNs(u64, u64),  // (before, after) - Duration in nanoseconds
    AllocBytes(u64, u64),  // (before, after) - Bytes allocated
    AllocCount(u64, u64),  // (before, after) - Allocation count
    AllocSigned(i64, i64), // (before, after) - Signed bytes
    Percentage(u64, u64),  // (before, after)
}

impl fmt::Display for MetricDiff {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.format_with_emoji(None))
    }
}

impl MetricDiff {
    pub fn format_with_emoji(&self, emoji_threshold: Option<u32>) -> String {
        match self {
            MetricDiff::CallsCount(before, after) => {
                let diff_percent = calculate_percentage_diff(*before, *after);
                let emoji = get_emoji_for_diff(diff_percent, emoji_threshold);
                format!("{} → {} ({:+.1}%){}", before, after, diff_percent, emoji)
            }
            MetricDiff::DurationNs(before, after) => {
                let diff_percent = calculate_percentage_diff(*before, *after);
                let before_duration = Duration::from_nanos(*before);
                let after_duration = Duration::from_nanos(*after);
                let emoji = get_emoji_for_diff(diff_percent, emoji_threshold);
                format!(
                    "{:.2?} → {:.2?} ({:+.1}%){}",
                    before_duration, after_duration, diff_percent, emoji
                )
            }
            MetricDiff::AllocBytes(before, after) => {
                let diff_percent = calculate_percentage_diff(*before, *after);
                let emoji = get_emoji_for_diff(diff_percent, emoji_threshold);
                format!(
                    "{} → {} ({:+.1}%){}",
                    format_bytes(*before),
                    format_bytes(*after),
                    diff_percent,
                    emoji
                )
            }
            MetricDiff::AllocCount(before, after) => {
                let diff_percent = calculate_percentage_diff(*before, *after);
                let emoji = get_emoji_for_diff(diff_percent, emoji_threshold);
                format!(
                    "{} → {} ({:+.1}%){}",
                    format_count(*before),
                    format_count(*after),
                    diff_percent,
                    emoji
                )
            }
            MetricDiff::AllocSigned(before, after) => {
                let diff_percent = calculate_percentage_diff_signed(*before, *after);
                let emoji = get_emoji_for_diff(diff_percent, emoji_threshold);
                format!(
                    "{} → {} ({:+.1}%){}",
                    format_bytes_signed(*before),
                    format_bytes_signed(*after),
                    diff_percent,
                    emoji
                )
            }
            MetricDiff::Percentage(before, after) => {
                let diff_percent = calculate_percentage_diff(*before, *after);
                let before_percent = *before as f64 / 100.0;
                let after_percent = *after as f64 / 100.0;
                let emoji = get_emoji_for_diff(diff_percent, emoji_threshold);
                format!(
                    "{:.2}% → {:.2}% ({:+.1}%){}",
                    before_percent, after_percent, diff_percent, emoji
                )
            }
        }
    }
}

fn get_emoji_for_diff(diff_percent: f64, threshold: Option<u32>) -> &'static str {
    if let Some(threshold_val) = threshold {
        let threshold = threshold_val as f64;
        if diff_percent > threshold {
            " ⚠️ "
        } else if diff_percent < -threshold {
            " 🚀 "
        } else {
            "   "
        }
    } else {
        ""
    }
}

#[derive(Debug, Clone)]
pub struct FunctionsComparison {
    pub profiling_mode: hotpath::ProfilingMode,
    pub description: String,
    pub percentiles: Vec<f64>,
    pub function_diffs: Vec<FunctionMetricsDiff>,
}

#[derive(Debug, Clone)]
pub struct FunctionMetricsDiff {
    pub function_name: String,
    pub metrics: Vec<MetricDiff>,
    pub is_removed: bool,
    pub is_new: bool,
}

#[derive(Debug, Clone)]
pub struct ThreadMetricsDiff {
    pub thread_name: String,
    pub cpu_percent_avg: Option<MetricDiff>,
    pub cpu_percent_max: Option<MetricDiff>,
    pub alloc_bytes: Option<MetricDiff>,
    pub dealloc_bytes: Option<MetricDiff>,
    pub mem_diff: Option<MetricDiff>,
    pub is_removed: bool,
    pub is_new: bool,
}

#[derive(Debug, Clone)]
pub struct ThreadsComparison {
    pub total_alloc_diff: Option<MetricDiff>,
    pub total_dealloc_diff: Option<MetricDiff>,
    pub total_mem_diff_diff: Option<MetricDiff>,
    pub thread_diffs: Vec<ThreadMetricsDiff>,
}

#[derive(Debug, Clone)]
pub struct JsonReportDiff {
    pub before_label: Option<String>,
    pub after_label: Option<String>,
    pub total_elapsed_diff: MetricDiff,
    pub functions_timing: Option<FunctionsComparison>,
    pub functions_alloc: Option<FunctionsComparison>,
    pub threads: Option<ThreadsComparison>,
}

fn ensure_matching_profiling_mode(
    section_name: &str,
    before_metrics: &JsonFunctionsList,
    after_metrics: &JsonFunctionsList,
) -> Result<(), String> {
    if before_metrics.profiling_mode != after_metrics.profiling_mode {
        return Err(format!(
            "Profiling mode mismatch in {} section: before={}, after={}",
            section_name, before_metrics.profiling_mode, after_metrics.profiling_mode
        ));
    }
    Ok(())
}

pub fn compare_reports(before: &JsonReport, after: &JsonReport) -> Result<JsonReportDiff, String> {
    if let (Some(b), Some(a)) = (&before.functions_timing, &after.functions_timing) {
        ensure_matching_profiling_mode("functions_timing", b, a)?;
    }
    if let (Some(b), Some(a)) = (&before.functions_alloc, &after.functions_alloc) {
        ensure_matching_profiling_mode("functions_alloc", b, a)?;
    }

    let functions_timing = match (&before.functions_timing, &after.functions_timing) {
        (Some(b), Some(a)) => Some(compare_metrics(b, a)),
        _ => None,
    };

    let functions_alloc = match (&before.functions_alloc, &after.functions_alloc) {
        (Some(b), Some(a)) => Some(compare_metrics(b, a)),
        _ => None,
    };

    let (before_section, after_section) = before
        .functions_timing
        .as_ref()
        .zip(after.functions_timing.as_ref())
        .or_else(|| {
            before
                .functions_alloc
                .as_ref()
                .zip(after.functions_alloc.as_ref())
        })
        .unzip();

    let before_ns = before_section.map(|s| s.total_elapsed_ns).unwrap_or(0);
    let after_ns = after_section.map(|s| s.total_elapsed_ns).unwrap_or(0);

    let threads = match (&before.threads, &after.threads) {
        (Some(b), Some(a)) => Some(compare_threads(b, a)),
        _ => None,
    };

    Ok(JsonReportDiff {
        before_label: before.label.clone(),
        after_label: after.label.clone(),
        total_elapsed_diff: MetricDiff::DurationNs(before_ns, after_ns),
        functions_timing,
        functions_alloc,
        threads,
    })
}

fn calculate_percentage_diff(before: u64, after: u64) -> f64 {
    if before == 0 {
        if after == 0 {
            0.0
        } else {
            100.0
        }
    } else {
        ((after as f64 - before as f64) / before as f64) * 100.0
    }
}

fn calculate_percentage_diff_signed(before: i64, after: i64) -> f64 {
    if before == 0 {
        if after == 0 {
            0.0
        } else {
            100.0
        }
    } else {
        ((after as f64 - before as f64) / (before as f64).abs()) * 100.0
    }
}

fn find_function<'a>(data: &'a [JsonFunctionEntry], name: &str) -> Option<&'a JsonFunctionEntry> {
    data.iter().find(|f| f.name == name)
}

fn parse_metric(s: &str, mode: &hotpath::ProfilingMode) -> Option<u64> {
    use hotpath::ProfilingMode;
    match mode {
        ProfilingMode::AllocCount => parse_count(s),
        ProfilingMode::AllocBytes => parse_bytes(s),
        ProfilingMode::Timing => parse_duration(s),
    }
}

fn parse_percent(s: &str) -> Option<u64> {
    let s = s.trim().trim_end_matches('%').trim();
    let pct: f64 = s.parse().ok()?;
    Some((pct * 100.0).round() as u64)
}

#[derive(Debug, Clone, Copy)]
enum MetricKind {
    Calls,
    Duration,
    Alloc,
    AllocCount,
    Percentage,
}

fn build_metrics_from_function(
    func: &JsonFunctionEntry,
    percentiles: &[f64],
    mode: &hotpath::ProfilingMode,
) -> Vec<(MetricKind, u64)> {
    use hotpath::ProfilingMode;

    let mut metrics = Vec::new();
    let kind = match mode {
        ProfilingMode::AllocCount => MetricKind::AllocCount,
        ProfilingMode::AllocBytes => MetricKind::Alloc,
        ProfilingMode::Timing => MetricKind::Duration,
    };

    metrics.push((MetricKind::Calls, func.calls));

    if let Some(val) = parse_metric(&func.avg, mode) {
        metrics.push((kind, val));
    }

    for p in percentiles {
        let key = hotpath::format_percentile_key(*p);
        if let Some(formatted) = func.percentiles.get(&key) {
            if let Some(val) = parse_metric(formatted, mode) {
                metrics.push((kind, val));
            }
        }
    }

    if let Some(val) = parse_metric(&func.total, mode) {
        metrics.push((kind, val));
    }

    if let Some(bp) = parse_percent(&func.percent_total) {
        metrics.push((MetricKind::Percentage, bp));
    }

    metrics
}

pub fn compare_metrics(
    before_metrics: &JsonFunctionsList,
    after_metrics: &JsonFunctionsList,
) -> FunctionsComparison {
    let mode = &before_metrics.profiling_mode;

    let mut function_diffs = Vec::new();
    let mut new_functions = Vec::new();

    let make_diff = |kind: &MetricKind, before_val: u64, after_val: u64| -> MetricDiff {
        match kind {
            MetricKind::Calls => MetricDiff::CallsCount(before_val, after_val),
            MetricKind::Duration => MetricDiff::DurationNs(before_val, after_val),
            MetricKind::Alloc => MetricDiff::AllocBytes(before_val, after_val),
            MetricKind::AllocCount => MetricDiff::AllocCount(before_val, after_val),
            MetricKind::Percentage => MetricDiff::Percentage(before_val, after_val),
        }
    };

    for after_func in &after_metrics.data {
        if let Some(before_func) = find_function(&before_metrics.data, &after_func.name) {
            let before_vals =
                build_metrics_from_function(before_func, &before_metrics.percentiles, mode);
            let after_vals =
                build_metrics_from_function(after_func, &after_metrics.percentiles, mode);

            let metrics = before_vals
                .iter()
                .zip(after_vals.iter())
                .map(|((kind, before_val), (_, after_val))| {
                    make_diff(kind, *before_val, *after_val)
                })
                .collect();

            function_diffs.push(FunctionMetricsDiff {
                function_name: after_func.name.clone(),
                metrics,
                is_removed: false,
                is_new: false,
            });
        } else {
            let after_vals =
                build_metrics_from_function(after_func, &after_metrics.percentiles, mode);

            let metrics = after_vals
                .iter()
                .map(|(kind, after_val)| make_diff(kind, 0, *after_val))
                .collect();

            new_functions.push(FunctionMetricsDiff {
                function_name: after_func.name.clone(),
                metrics,
                is_removed: false,
                is_new: true,
            });
        }
    }

    for before_func in &before_metrics.data {
        if find_function(&after_metrics.data, &before_func.name).is_none() {
            let before_vals =
                build_metrics_from_function(before_func, &before_metrics.percentiles, mode);

            let metrics = before_vals
                .iter()
                .map(|(kind, before_val)| make_diff(kind, *before_val, 0))
                .collect();

            function_diffs.push(FunctionMetricsDiff {
                function_name: before_func.name.clone(),
                metrics,
                is_removed: true,
                is_new: false,
            });
        }
    }

    function_diffs.extend(new_functions);

    function_diffs.sort_by(|a, b| {
        let a_percent = a
            .metrics
            .iter()
            .find_map(|m| {
                if let MetricDiff::Percentage(_, after) = m {
                    Some(*after)
                } else {
                    None
                }
            })
            .unwrap_or(0);

        let b_percent = b
            .metrics
            .iter()
            .find_map(|m| {
                if let MetricDiff::Percentage(_, after) = m {
                    Some(*after)
                } else {
                    None
                }
            })
            .unwrap_or(0);

        b_percent.cmp(&a_percent)
    });

    FunctionsComparison {
        profiling_mode: before_metrics.profiling_mode,
        description: before_metrics.description.clone(),
        percentiles: before_metrics.percentiles.clone(),
        function_diffs,
    }
}

fn parse_cpu_percent(s: &str) -> Option<u64> {
    let s = s.trim().trim_end_matches('%').trim();
    let pct: f64 = s.parse().ok()?;
    Some((pct * 100.0).round() as u64)
}

fn find_thread<'a>(data: &'a [JsonThreadEntry], name: &str) -> Option<&'a JsonThreadEntry> {
    data.iter().find(|t| t.name == name)
}

fn make_percent_diff(before: &Option<String>, after: &Option<String>) -> Option<MetricDiff> {
    let b = parse_cpu_percent(before.as_deref()?)?;
    let a = parse_cpu_percent(after.as_deref()?)?;
    Some(MetricDiff::Percentage(b, a))
}

fn make_alloc_diff(before: &Option<String>, after: &Option<String>) -> Option<MetricDiff> {
    let b = parse_bytes(before.as_deref()?)?;
    let a = parse_bytes(after.as_deref()?)?;
    Some(MetricDiff::AllocBytes(b, a))
}

fn make_alloc_signed_diff(before: &Option<String>, after: &Option<String>) -> Option<MetricDiff> {
    let b = parse_bytes_signed(before.as_deref()?)?;
    let a = parse_bytes_signed(after.as_deref()?)?;
    Some(MetricDiff::AllocSigned(b, a))
}

fn build_thread_diff(
    before: &JsonThreadEntry,
    after: &JsonThreadEntry,
    is_new: bool,
    is_removed: bool,
) -> ThreadMetricsDiff {
    ThreadMetricsDiff {
        thread_name: if is_new {
            after.name.clone()
        } else {
            before.name.clone()
        },
        cpu_percent_avg: make_percent_diff(&before.cpu_percent_avg, &after.cpu_percent_avg),
        cpu_percent_max: make_percent_diff(&before.cpu_percent_max, &after.cpu_percent_max),
        alloc_bytes: make_alloc_diff(&before.alloc_bytes, &after.alloc_bytes),
        dealloc_bytes: make_alloc_diff(&before.dealloc_bytes, &after.dealloc_bytes),
        mem_diff: make_alloc_signed_diff(&before.mem_diff, &after.mem_diff),
        is_removed,
        is_new,
    }
}

pub fn compare_threads(
    before_threads: &JsonThreadsList,
    after_threads: &JsonThreadsList,
) -> ThreadsComparison {
    let mut thread_diffs = Vec::new();
    let mut new_threads = Vec::new();

    let zero = JsonThreadEntry {
        os_tid: 0,
        name: String::new(),
        status: String::new(),
        status_code: String::new(),
        cpu_percent: None,
        cpu_percent_max: Some("0.0%".to_string()),
        cpu_percent_avg: Some("0.0%".to_string()),
        alloc_bytes: Some("0 B".to_string()),
        dealloc_bytes: Some("0 B".to_string()),
        mem_diff: Some("0 B".to_string()),
    };

    let duplicate_names = {
        let mut dups = std::collections::HashSet::new();
        for list in [&before_threads.data, &after_threads.data] {
            let mut counts = std::collections::HashMap::<&str, usize>::new();
            for t in list {
                *counts.entry(&t.name).or_default() += 1;
            }
            for (name, count) in counts {
                if count > 1 {
                    dups.insert(name.to_string());
                }
            }
        }
        dups
    };

    let before_data: Vec<_> = before_threads
        .data
        .iter()
        .filter(|t| !duplicate_names.contains(&t.name))
        .collect();
    let after_data: Vec<_> = after_threads
        .data
        .iter()
        .filter(|t| !duplicate_names.contains(&t.name))
        .collect();

    for after_thread in &after_data {
        if let Some(before_thread) = find_thread(&before_threads.data, &after_thread.name) {
            thread_diffs.push(build_thread_diff(before_thread, after_thread, false, false));
        } else {
            new_threads.push(build_thread_diff(&zero, after_thread, true, false));
        }
    }

    for before_thread in &before_data {
        if find_thread(&after_threads.data, &before_thread.name).is_none() {
            thread_diffs.push(build_thread_diff(before_thread, &zero, false, true));
        }
    }

    thread_diffs.extend(new_threads);

    let total_alloc_diff = make_alloc_diff(
        &before_threads.total_alloc_bytes,
        &after_threads.total_alloc_bytes,
    );
    let total_dealloc_diff = make_alloc_diff(
        &before_threads.total_dealloc_bytes,
        &after_threads.total_dealloc_bytes,
    );
    let total_mem_diff_diff = make_alloc_signed_diff(
        &before_threads.alloc_dealloc_diff,
        &after_threads.alloc_dealloc_diff,
    );

    ThreadsComparison {
        total_alloc_diff,
        total_dealloc_diff,
        total_mem_diff_diff,
        thread_diffs,
    }
}

pub fn build_functions_table(
    comparison: &FunctionsComparison,
    emoji_threshold: Option<u32>,
) -> Table {
    let mut table = Table::new();

    let mut header_cells = vec![
        Cell::header("Function"),
        Cell::header("Calls"),
        Cell::header("Avg"),
    ];
    for &p in &comparison.percentiles {
        header_cells.push(Cell::header(&hotpath::format_percentile_header(p)));
    }
    header_cells.push(Cell::header("Total"));
    header_cells.push(Cell::header("% Total"));
    table.add_row(header_cells);

    for func_diff in &comparison.function_diffs {
        let short_name = shorten_function_name(&func_diff.function_name);
        let function_display = if func_diff.is_removed {
            format!("🗑️ {}", short_name)
        } else if func_diff.is_new {
            format!("🆕 {}", short_name)
        } else {
            short_name
        };

        let mut row_cells = vec![Cell::new(&function_display)];
        for metric_diff in &func_diff.metrics {
            row_cells.push(Cell::new(&metric_diff.format_with_emoji(emoji_threshold)));
        }
        table.add_row(row_cells);
    }

    table
}

pub fn build_threads_table(threads: &ThreadsComparison, emoji_threshold: Option<u32>) -> Table {
    let fmt = |m: &Option<MetricDiff>| {
        m.as_ref()
            .map(|d| d.format_with_emoji(emoji_threshold))
            .unwrap_or_default()
    };

    let has_alloc = threads
        .thread_diffs
        .iter()
        .any(|d| d.alloc_bytes.is_some() || d.dealloc_bytes.is_some() || d.mem_diff.is_some());

    let mut table = Table::new();
    let mut header = vec![
        Cell::header("Thread"),
        Cell::header("CPU % Avg"),
        Cell::header("CPU % Max"),
    ];
    if has_alloc {
        header.extend([
            Cell::header("Alloc"),
            Cell::header("Dealloc"),
            Cell::header("Mem Diff"),
        ]);
    }
    table.add_row(header);

    for diff in &threads.thread_diffs {
        let name = if diff.is_removed {
            format!("🗑️ {}", diff.thread_name)
        } else if diff.is_new {
            format!("🆕 {}", diff.thread_name)
        } else {
            diff.thread_name.clone()
        };

        let mut row = vec![
            Cell::new(&name),
            Cell::new(&fmt(&diff.cpu_percent_avg)),
            Cell::new(&fmt(&diff.cpu_percent_max)),
        ];
        if has_alloc {
            row.extend([
                Cell::new(&fmt(&diff.alloc_bytes)),
                Cell::new(&fmt(&diff.dealloc_bytes)),
                Cell::new(&fmt(&diff.mem_diff)),
            ]);
        }
        table.add_row(row);
    }

    table
}

pub fn format_threads_globals(
    threads: &ThreadsComparison,
    emoji_threshold: Option<u32>,
) -> Vec<String> {
    let fmt = |label: &str, m: &Option<MetricDiff>| {
        m.as_ref()
            .map(|d| format!("{}: {}", label, d.format_with_emoji(emoji_threshold)))
    };

    [
        fmt("Total Alloc", &threads.total_alloc_diff),
        fmt("Total Dealloc", &threads.total_dealloc_diff),
        fmt("Mem Diff", &threads.total_mem_diff_diff),
    ]
    .into_iter()
    .flatten()
    .collect()
}

#[cfg(test)]
mod test {
    use crate::cmd::shared::{compare_metrics, compare_reports, compare_threads};
    use hotpath::json::{
        JsonFunctionEntry, JsonFunctionsList, JsonMeta, JsonReport, JsonThreadEntry,
        JsonThreadsList,
    };
    use std::collections::HashMap;
    use time::macros::datetime;

    fn make_function_data(
        name: &str,
        calls: u64,
        avg: u64,
        p95: u64,
        total: u64,
        percent: u64,
    ) -> JsonFunctionEntry {
        let mut percentiles = HashMap::new();
        percentiles.insert("p95".to_string(), hotpath::format_duration(p95));

        JsonFunctionEntry {
            id: 0,
            name: name.to_string(),
            calls,
            sampled_calls: calls,
            avg: hotpath::format_duration(avg),
            percentiles,
            total: hotpath::format_duration(total),
            percent_total: format!("{:.2}%", percent as f64 / 100.0),
            location: None,
        }
    }

    fn make_metrics(data: Vec<JsonFunctionEntry>, total_elapsed_ns: u64) -> JsonFunctionsList {
        let count = data.len();
        JsonFunctionsList {
            profiling_mode: hotpath::ProfilingMode::Timing,
            time_elapsed: hotpath::format_duration(total_elapsed_ns),
            total_elapsed_ns,
            total_allocated: None,
            description: "Time metrics".to_string(),
            caller_name: "test::main".to_string(),
            percentiles: vec![95.0],
            included_count: count,
            total_count: count,
            data,
        }
    }

    fn make_alloc_function_data(
        name: &str,
        calls: u64,
        avg_bytes: u64,
        p95_bytes: u64,
        total_bytes: u64,
        percent: u64,
    ) -> JsonFunctionEntry {
        let mut percentiles = HashMap::new();
        percentiles.insert("p95".to_string(), hotpath::format_bytes(p95_bytes));

        JsonFunctionEntry {
            id: 0,
            name: name.to_string(),
            calls,
            sampled_calls: calls,
            avg: hotpath::format_bytes(avg_bytes),
            percentiles,
            total: hotpath::format_bytes(total_bytes),
            percent_total: format!("{:.2}%", percent as f64 / 100.0),
            location: None,
        }
    }

    fn make_alloc_metrics(
        data: Vec<JsonFunctionEntry>,
        total_elapsed_ns: u64,
    ) -> JsonFunctionsList {
        let count = data.len();
        JsonFunctionsList {
            profiling_mode: hotpath::ProfilingMode::AllocBytes,
            time_elapsed: hotpath::format_duration(total_elapsed_ns),
            total_elapsed_ns,
            total_allocated: Some("10.00 MB".to_string()),
            description: "Alloc metrics".to_string(),
            caller_name: "test::main".to_string(),
            percentiles: vec![95.0],
            included_count: count,
            total_count: count,
            data,
        }
    }

    fn make_report(
        timing: Option<JsonFunctionsList>,
        alloc: Option<JsonFunctionsList>,
    ) -> JsonReport {
        JsonReport {
            functions_timing: timing,
            functions_alloc: alloc,
            ..JsonReport::new(JsonMeta {
                rustc: "1.89.0".to_string(),
                os: "linux-x86_64".to_string(),
                created_at: datetime!(2026-08-27 10:15:42 UTC),
                source_root: None,
                git: None,
                ci: None,
                benchmark: None,
                policy: None,
            })
        }
    }

    #[test]
    fn test_compare_metrics_new_removed_unchanged() {
        let after_data = vec![
            make_function_data("test::function_a", 100, 1000000, 1100000, 100000000, 7000),
            make_function_data("test::function_c", 40, 400000, 450000, 16000000, 1500),
        ];
        let after_metrics = make_metrics(after_data, 140000000);

        let before_data = vec![
            make_function_data("test::function_a", 90, 900000, 1000000, 81000000, 8000),
            make_function_data("test::function_b", 30, 300000, 350000, 9000000, 1200),
        ];
        let before_metrics = make_metrics(before_data, 120000000);

        let comparison = compare_metrics(&before_metrics, &after_metrics);

        assert_eq!(comparison.function_diffs.len(), 3);
        assert!(comparison
            .function_diffs
            .iter()
            .any(|f| f.function_name == "test::function_b" && f.is_removed));
        assert!(comparison
            .function_diffs
            .iter()
            .any(|f| f.function_name == "test::function_c" && f.is_new));
        assert!(comparison
            .function_diffs
            .iter()
            .any(|f| f.function_name == "test::function_a" && !f.is_new && !f.is_removed));
    }

    #[test]
    fn test_compare_reports_timing_and_alloc() {
        let before_timing = make_metrics(
            vec![make_function_data(
                "fn_a", 100, 1000000, 1100000, 100000000, 10000,
            )],
            100000000,
        );
        let after_timing = make_metrics(
            vec![make_function_data(
                "fn_a", 120, 1200000, 1300000, 144000000, 10000,
            )],
            144000000,
        );
        let before_alloc = make_alloc_metrics(
            vec![make_alloc_function_data(
                "fn_a", 100, 1024, 2048, 102400, 10000,
            )],
            100000000,
        );
        let after_alloc = make_alloc_metrics(
            vec![make_alloc_function_data(
                "fn_a", 120, 2048, 4096, 245760, 10000,
            )],
            144000000,
        );

        let before = make_report(Some(before_timing), Some(before_alloc));
        let after = make_report(Some(after_timing), Some(after_alloc));

        let diff = compare_reports(&before, &after).expect("reports should compare");

        assert!(diff.functions_timing.is_some());
        assert!(diff.functions_alloc.is_some());

        let timing = diff.functions_timing.unwrap();
        assert_eq!(timing.function_diffs.len(), 1);
        assert_eq!(timing.function_diffs[0].function_name, "fn_a");

        let alloc = diff.functions_alloc.unwrap();
        assert_eq!(alloc.function_diffs.len(), 1);
        assert_eq!(alloc.function_diffs[0].function_name, "fn_a");
    }

    fn make_thread_entry(os_tid: u64, name: &str, cpu_percent_max: f64) -> JsonThreadEntry {
        JsonThreadEntry {
            os_tid,
            name: name.to_string(),
            status: "Running".to_string(),
            status_code: "1".to_string(),
            cpu_percent: None,
            cpu_percent_max: Some(format!("{:.1}%", cpu_percent_max)),
            cpu_percent_avg: None,
            alloc_bytes: None,
            dealloc_bytes: None,
            mem_diff: None,
        }
    }

    fn make_threads_list(data: Vec<JsonThreadEntry>) -> JsonThreadsList {
        let thread_count = data.len();
        JsonThreadsList {
            current_elapsed_ns: 1_000_000_000,
            sample_interval_ms: 1000,
            thread_count,
            total_count: thread_count,
            included_count: thread_count,
            rss_bytes: None,
            rss_bytes_max: None,
            total_alloc_bytes: None,
            total_dealloc_bytes: None,
            alloc_dealloc_diff: None,
            pmu: None,
            data,
        }
    }

    #[test]
    fn test_compare_threads_new_removed_unchanged() {
        let before = make_threads_list(vec![
            make_thread_entry(100, "main", 30.0),
            make_thread_entry(101, "worker-1", 17.5),
        ]);
        let after = make_threads_list(vec![
            make_thread_entry(200, "main", 42.5),
            make_thread_entry(201, "worker-2", 11.5),
        ]);

        let comparison = compare_threads(&before, &after);

        assert_eq!(comparison.thread_diffs.len(), 3);
        assert!(comparison
            .thread_diffs
            .iter()
            .any(|t| t.thread_name == "main" && !t.is_new && !t.is_removed));
        assert!(comparison
            .thread_diffs
            .iter()
            .any(|t| t.thread_name == "worker-1" && t.is_removed));
        assert!(comparison
            .thread_diffs
            .iter()
            .any(|t| t.thread_name == "worker-2" && t.is_new));

        assert!(comparison.total_alloc_diff.is_none());
        assert!(comparison.total_dealloc_diff.is_none());
        assert!(comparison.total_mem_diff_diff.is_none());
    }

    #[test]
    fn test_compare_threads_global_alloc_diffs() {
        let mut before = make_threads_list(vec![make_thread_entry(100, "main", 30.0)]);
        before.total_alloc_bytes = Some("1.00 MB".to_string());
        before.total_dealloc_bytes = Some("512.00 KB".to_string());
        before.alloc_dealloc_diff = Some("512.00 KB".to_string());

        let mut after = make_threads_list(vec![make_thread_entry(200, "main", 42.5)]);
        after.total_alloc_bytes = Some("2.00 MB".to_string());
        after.total_dealloc_bytes = Some("1.00 MB".to_string());
        after.alloc_dealloc_diff = Some("1.00 MB".to_string());

        let comparison = compare_threads(&before, &after);

        assert!(comparison.total_alloc_diff.is_some());
        assert!(comparison.total_dealloc_diff.is_some());
        assert!(comparison.total_mem_diff_diff.is_some());
    }

    #[test]
    fn test_compare_threads_duplicate_names_skipped() {
        let before = make_threads_list(vec![
            make_thread_entry(100, "main", 30.0),
            make_thread_entry(101, "worker", 10.0),
            make_thread_entry(102, "worker", 20.0),
        ]);
        let after = make_threads_list(vec![
            make_thread_entry(200, "main", 42.5),
            make_thread_entry(201, "worker", 15.0),
            make_thread_entry(202, "worker", 25.0),
        ]);

        let comparison = compare_threads(&before, &after);

        assert_eq!(comparison.thread_diffs.len(), 1);
        assert!(comparison
            .thread_diffs
            .iter()
            .any(|t| t.thread_name == "main" && !t.is_new && !t.is_removed));
        assert!(!comparison
            .thread_diffs
            .iter()
            .any(|t| t.thread_name == "worker"));
    }
}

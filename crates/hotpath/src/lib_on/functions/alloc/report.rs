use std::collections::HashMap;

use crate::json::JsonFunctionEntry;
use crate::json::JsonFunctionsList;
use crate::lib_on::functions::FunctionStatsConfig;
use crate::output::{format_count, format_percentile_key, Precision, ProfilingMode};

use crate::lib_on::functions::alloc::state::FunctionStats;

pub(crate) fn build_functions_list_alloc(
    stats: &HashMap<u32, FunctionStats>,
    config: &FunctionStatsConfig,
    current_elapsed_ns: u64,
) -> JsonFunctionsList {
    use crate::lib_on::functions::alloc::guard::{AllocMetric, ALLOC_METRIC};

    let exclude_wrapper = *crate::functions::EXCLUDE_WRAPPER;
    let use_count = *ALLOC_METRIC == AllocMetric::Count;
    let precision = Precision::for_cloud(config.cloud);

    let bytes_cache: HashMap<u32, u64> = stats
        .iter()
        .filter(|(_, s)| s.has_data)
        .map(|(&id, s)| (id, s.total_bytes()))
        .collect();

    let count_cache: HashMap<u32, u64> = stats
        .iter()
        .filter(|(_, s)| s.has_data)
        .map(|(&id, s)| (id, s.total_count()))
        .collect();

    let primary_cache = if use_count {
        &count_cache
    } else {
        &bytes_cache
    };

    let grand_total: u64 = if *crate::functions::EXCLUDE_WRAPPER {
        stats
            .values()
            .filter(|s| !s.wrapper && s.has_data)
            .map(|s| primary_cache.get(&s.id).copied().unwrap_or(0))
            .sum()
    } else if *crate::lib_on::functions::alloc::guard::ALLOC_CUMULATIVE {
        let wrapper_total = stats
            .values()
            .find(|s| s.wrapper && s.has_data)
            .map(|s| primary_cache.get(&s.id).copied().unwrap_or(0));

        wrapper_total.unwrap_or_else(|| {
            stats
                .values()
                .filter(|s| s.has_data)
                .map(|s| primary_cache.get(&s.id).copied().unwrap_or(0))
                .sum()
        })
    } else {
        stats
            .values()
            .filter(|s| s.has_data)
            .map(|s| primary_cache.get(&s.id).copied().unwrap_or(0))
            .sum()
    };

    let mut entries: Vec<_> = stats
        .values()
        .filter(|s| s.has_data && !(exclude_wrapper && s.wrapper))
        .collect();

    entries.sort_by(|a, b| {
        let a_primary = primary_cache.get(&a.id).copied().unwrap_or(0);
        let b_primary = primary_cache.get(&b.id).copied().unwrap_or(0);
        b_primary.cmp(&a_primary).then_with(|| a.name.cmp(b.name))
    });

    let total_count = entries.len();
    let included_count = if config.limit > 0 && config.limit < total_count {
        config.limit
    } else {
        total_count
    };

    if config.limit > 0 {
        entries.truncate(config.limit);
    }

    let format_alloc_value = |bytes: u64, count: u64| -> String {
        if use_count {
            format_count(count)
        } else {
            precision.bytes(bytes)
        }
    };

    let profiling_mode = match *ALLOC_METRIC {
        AllocMetric::Bytes => ProfilingMode::AllocBytes,
        AllocMetric::Count => ProfilingMode::AllocCount,
    };

    let data: Vec<JsonFunctionEntry> = entries
        .into_iter()
        .map(|s| {
            let entry_bytes = bytes_cache.get(&s.id).copied().unwrap_or(0);
            let entry_count = count_cache.get(&s.id).copied().unwrap_or(0);
            let primary_total = if use_count { entry_count } else { entry_bytes };

            let percentage = if grand_total > 0 {
                (primary_total as f64 / grand_total as f64) * 100.0
            } else {
                0.0
            };

            let (avg, total, percent_total) = if s.is_async {
                ("N/A".to_string(), "N/A".to_string(), "N/A".to_string())
            } else {
                (
                    format_alloc_value(s.avg_bytes(), s.avg_count()),
                    format_alloc_value(entry_bytes, entry_count),
                    format!("{:.2}%", percentage),
                )
            };

            let mut percentiles = HashMap::new();
            for &p in &config.percentiles {
                if s.is_async {
                    percentiles.insert(format_percentile_key(p), "N/A".to_string());
                } else {
                    let bytes_total = s.bytes_total_percentile(p);
                    let count_total = s.count_total_percentile(p);
                    percentiles.insert(
                        format_percentile_key(p),
                        format_alloc_value(bytes_total, count_total),
                    );
                }
            }

            JsonFunctionEntry {
                id: s.id,
                name: s.name.to_string(),
                calls: s.count,
                sampled_calls: s.count,
                avg,
                percentiles,
                total,
                percent_total,
                location: crate::lib_on::locations::lookup_location(s.name),
            }
        })
        .collect();

    let description = {
        let metric = match *ALLOC_METRIC {
            AllocMetric::Bytes => "bytes",
            AllocMetric::Count => "count",
        };
        if *crate::lib_on::functions::alloc::guard::ALLOC_CUMULATIVE {
            format!(
                "Cumulative allocation {} during each function call (including nested calls).",
                metric
            )
        } else {
            format!(
                "Exclusive allocation {} by each function (excluding nested calls).",
                metric
            )
        }
    };

    JsonFunctionsList {
        profiling_mode,
        time_elapsed: precision.duration(current_elapsed_ns),
        total_elapsed_ns: current_elapsed_ns,
        total_allocated: match profiling_mode {
            ProfilingMode::AllocBytes => Some(precision.bytes(grand_total)),
            ProfilingMode::AllocCount => Some(format_count(grand_total)),
            ProfilingMode::Timing => None,
        },
        description,
        caller_name: config.caller_name.to_string(),
        percentiles: config.percentiles.clone(),
        data,
        included_count,
        total_count,
    }
}

pub(crate) fn build_functions_list_timing(
    stats: &HashMap<u32, FunctionStats>,
    config: &FunctionStatsConfig,
    current_elapsed_ns: u64,
) -> JsonFunctionsList {
    let exclude_wrapper = *crate::functions::EXCLUDE_WRAPPER;
    let precision = Precision::for_cloud(config.cloud);

    let time_exclusive = *crate::lib_on::functions::exclusive::TIME_EXCLUSIVE;

    let reference_total = if exclude_wrapper {
        stats
            .values()
            .filter(|s| !s.wrapper && s.has_data)
            .map(|s| s.display_total_duration_ns())
            .sum::<u64>()
    } else if time_exclusive {
        // The wrapper row holds only its own time here, so the elapsed time
        // stands in for its inclusive total.
        config.total_elapsed.as_nanos() as u64
    } else {
        // The wrapper guard is exempt from time sampling, so its total is exact.
        let wrapper_total = stats
            .values()
            .find(|s| s.wrapper)
            .map(|s| s.total_duration_ns);
        wrapper_total.unwrap_or(config.total_elapsed.as_nanos() as u64)
    };

    let mut entries: Vec<_> = stats
        .values()
        .filter(|s| s.has_data && !(exclude_wrapper && s.wrapper))
        .collect();

    entries.sort_by(|a, b| {
        b.display_total_duration_ns()
            .cmp(&a.display_total_duration_ns())
            .then_with(|| a.name.cmp(b.name))
    });

    let total_count = entries.len();
    let included_count = if config.limit > 0 && config.limit < total_count {
        config.limit
    } else {
        total_count
    };

    if config.limit > 0 {
        entries.truncate(config.limit);
    }

    let data: Vec<JsonFunctionEntry> = entries
        .into_iter()
        .map(|s| {
            let count_only = s.duration_sampled_count == 0 && s.count > 0;
            let display_total = s.display_total_duration_ns();
            let percentage = if reference_total > 0 {
                (display_total as f64 / reference_total as f64) * 100.0
            } else {
                0.0
            };

            let mut percentiles = HashMap::new();
            for &p in &config.percentiles {
                let value = if count_only {
                    "-".to_string()
                } else {
                    precision.duration(s.duration_percentile(p))
                };
                percentiles.insert(format_percentile_key(p), value);
            }

            JsonFunctionEntry {
                id: s.id,
                name: s.name.to_string(),
                calls: s.count,
                sampled_calls: s.duration_sampled_count,
                avg: if count_only {
                    "-".to_string()
                } else {
                    precision.duration(s.avg_duration_ns())
                },
                percentiles,
                total: if count_only {
                    "-".to_string()
                } else {
                    precision.duration(display_total)
                },
                percent_total: if count_only {
                    "-".to_string()
                } else {
                    format!("{:.2}%", percentage)
                },
                location: crate::lib_on::locations::lookup_location(s.name),
            }
        })
        .collect();

    let total_elapsed_ns = config.total_elapsed.as_nanos() as u64;

    JsonFunctionsList {
        profiling_mode: ProfilingMode::Timing,
        time_elapsed: precision.duration(total_elapsed_ns),
        total_elapsed_ns: current_elapsed_ns,
        total_allocated: None,
        description: if time_exclusive {
            crate::lib_on::functions::exclusive::DESCRIPTION.to_string()
        } else {
            "Function execution time metrics.".to_string()
        },
        caller_name: config.caller_name.to_string(),
        percentiles: config.percentiles.clone(),
        data,
        included_count,
        total_count,
    }
}

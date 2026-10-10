use std::collections::HashMap;

use crate::json::JsonFunctionEntry;
use crate::json::JsonFunctionsList;
use crate::lib_on::functions::FunctionStatsConfig;
use crate::output::{format_percentile_key, Precision, ProfilingMode};

use crate::lib_on::functions::timing::state::FunctionStats;

pub(crate) fn build_functions_list(
    stats: &HashMap<u32, FunctionStats>,
    config: &FunctionStatsConfig,
    current_elapsed_ns: u64,
) -> JsonFunctionsList {
    let exclude_wrapper = *crate::functions::EXCLUDE_WRAPPER;
    let precision = Precision::for_cloud(config.cloud);

    let time_exclusive = *crate::lib_on::functions::exclusive::TIME_EXCLUSIVE;

    // Exclusive times of all rows (wrapper included) partition the measured
    // time, so their sum is the reference and the rows add up to 100%.
    let reference_total = if exclude_wrapper || time_exclusive {
        stats
            .values()
            .filter(|s| s.has_data && !(exclude_wrapper && s.wrapper))
            .map(|s| s.display_total_ns())
            .sum::<u64>()
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
        b.display_total_ns()
            .cmp(&a.display_total_ns())
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
            let count_only = s.sampled_count == 0 && s.count > 0;
            let display_total = s.display_total_ns();
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
                    precision.duration(s.percentile(p).as_nanos() as u64)
                };
                percentiles.insert(format_percentile_key(p), value);
            }

            JsonFunctionEntry {
                id: s.id,
                name: s.name.to_string(),
                calls: s.count,
                sampled_calls: s.sampled_count,
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
            "Execution duration of functions.".to_string()
        },
        caller_name: config.caller_name.to_string(),
        percentiles: config.percentiles.clone(),
        data,
        included_count,
        total_count,
    }
}

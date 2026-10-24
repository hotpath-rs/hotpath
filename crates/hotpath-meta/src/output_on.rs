use crate::json::JsonFunctionsList;
use crate::output::{
    floor_char_boundary, format_bytes, format_count_display, format_percentile_header,
    format_percentile_key, shorten_function_name, MAX_LOG_LEN,
};
use crate::shared::Section;
use crate::table::{Cell, Table};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::Duration;

pub(crate) fn write_report_header<W: Write + ?Sized>(
    writer: &mut W,
    elapsed: Duration,
    sections: &[Section],
    label: Option<&str>,
) {
    let section_names: Vec<&str> = sections.iter().map(|s| s.short_name()).collect();
    let sections_str = section_names.join(", ");
    let label_str = label.map(|l| format!(" | {}", l)).unwrap_or_default();
    let sampling_str = crate::lib_on::sampling::active_rates()
        .map(|rates| {
            let mut parts: Vec<String> = rates
                .iter()
                .map(|(name, rate)| format!("{}={}", name, rate))
                .collect();
            parts.sort();
            format!(" | time sampling: {}", parts.join(", "))
        })
        .unwrap_or_default();

    let _ = writeln!(
        writer,
        "[hotpath-meta] {:.2?} | {}{}{}",
        elapsed, sections_str, label_str, sampling_str,
    );
    let _ = writeln!(writer);
}

/// `HOTPATH_META_DISABLE_HINTS=1` hides the hint line under the table report.
static DISABLE_HINTS: LazyLock<bool> = LazyLock::new(|| {
    std::env::var("HOTPATH_META_DISABLE_HINTS")
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true"))
        .unwrap_or(false)
});

/// A set `HOTPATH_META_API_TOKEN` marks an existing hotpath.rs user, who
/// needs no cloud hint.
static HAS_API_TOKEN: LazyLock<bool> =
    LazyLock::new(|| std::env::var("HOTPATH_META_API_TOKEN").is_ok_and(|v| !v.trim().is_empty()));

/// Hint line closing a table report written to stdout. Skipped for file
/// output, when the report is uploaded to hotpath.rs, or a token is set.
pub(crate) fn write_report_footer<W: Write + ?Sized>(
    writer: &mut W,
    output: &OutputDestination,
    upload_enabled: bool,
) {
    if *DISABLE_HINTS
        || upload_enabled
        || *HAS_API_TOKEN
        || !matches!(output, OutputDestination::Stdout)
    {
        return;
    }
    let _ = writeln!(
        writer,
        "└─ Prevent performance regressions in CI → https://hotpath.rs/cloud"
    );
}

pub(crate) fn write_user_metadata_table<W: Write>(
    writer: &mut W,
    metadata: &BTreeMap<String, String>,
) {
    if metadata.is_empty() {
        return;
    }

    let mut table = Table::new();
    table.add_row(vec![Cell::header("Key"), Cell::header("Value")]);
    for (key, value) in metadata {
        table.add_row(vec![Cell::new(key), Cell::new(value)]);
    }

    write_section_header(writer, "user_metadata", "Custom report metadata.");
    let _ = writeln!(writer);
    let _ = table.print(writer, use_colors());
    let _ = writeln!(writer);
}

pub(crate) fn write_section_header<W: Write + ?Sized>(
    writer: &mut W,
    section_name: &str,
    description: &str,
) {
    let _ = write!(writer, "{} - {}", section_name, description);
}

fn print_table<W: Write>(table: &Table, writer: &mut W) {
    let _ = table.print(writer, use_colors());
}

pub(crate) fn display_functions_table_to<W: Write>(writer: &mut W, list: &JsonFunctionsList) {
    let mut table = Table::new();

    let mut header_names = vec![
        "Function".to_string(),
        "Calls".to_string(),
        "Avg".to_string(),
    ];
    for &p in &list.percentiles {
        header_names.push(format_percentile_header(p));
    }
    header_names.push("Total".to_string());
    header_names.push("% Total".to_string());

    let header_cells: Vec<Cell> = header_names
        .iter()
        .map(|header| Cell::header(header))
        .collect();

    table.add_row(header_cells);

    let value = |s: &str| list.profiling_mode.display_value(s);

    for entry in &list.data {
        let mut row_cells = Vec::new();

        let short_name = shorten_function_name(&entry.name);
        row_cells.push(Cell::new(&short_name));
        row_cells.push(Cell::new(&format_count_display(entry.calls)));
        row_cells.push(Cell::new(&value(&entry.avg)));

        for &p in &list.percentiles {
            let key = format_percentile_key(p);
            let value = entry
                .percentiles
                .get(&key)
                .map_or_else(|| "N/A".to_string(), |s| value(s));
            row_cells.push(Cell::new(&value));
        }

        row_cells.push(Cell::new(&value(&entry.total)));
        row_cells.push(Cell::new(&entry.percent_total));

        table.add_row(row_cells);
    }

    let mode = list.profiling_mode.to_string();
    let desc = &list.description;
    if list.included_count < list.total_count {
        let _ = writeln!(
            writer,
            "{} - {} ({}/{})",
            mode, desc, list.included_count, list.total_count
        );
    } else {
        write_section_header(writer, &mode, desc);
        let _ = writeln!(writer);
    }

    if let Some(total_alloc) = &list.total_allocated {
        let _ = writeln!(writer, "Total: {}", value(total_alloc));
    }

    print_table(&table, writer);

    let _ = writeln!(writer);
}

pub(crate) fn display_no_measurements_message_to<W: Write>(
    writer: &mut W,
    total_elapsed: Duration,
    caller_name: &str,
) {
    let _ = writeln!(
        writer,
        "\n[hotpath-meta] No measurements recorded from {} (Total time: {:.2?})",
        caller_name, total_elapsed
    );
    let _ = writeln!(writer);
    let _ = writeln!(
        writer,
        "To start measuring performance, add the #[hotpath_meta::measure] macro to your functions:"
    );
    let _ = writeln!(writer);
    let _ = writeln!(writer, "  #[hotpath_meta::measure]");
    let _ = writeln!(writer, "  fn your_function() {{");
    let _ = writeln!(writer, "      // your code here");
    let _ = writeln!(writer, "  }}");
    let _ = writeln!(writer);
}

/// Destination for profiling report output.
#[derive(Default)]
pub(crate) enum OutputDestination {
    #[default]
    Stdout,
    File(PathBuf),
}

impl OutputDestination {
    /// For `File`, creates parent directories if needed, then creates or truncates the file.
    pub(crate) fn writer(&self) -> Result<Box<dyn Write>, std::io::Error> {
        match self {
            OutputDestination::Stdout => Ok(Box::new(std::io::stdout())),
            OutputDestination::File(path) => {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                Ok(Box::new(File::create(path)?))
            }
        }
    }

    /// `HOTPATH_META_OUTPUT_PATH` takes precedence over `path`. Relative paths
    /// resolve against the cwd; no path means stdout.
    pub(crate) fn from_path(path: Option<PathBuf>) -> Self {
        if let Ok(env_path) = std::env::var("HOTPATH_META_OUTPUT_PATH") {
            return OutputDestination::File(resolve_output_path(env_path));
        }

        match path {
            Some(p) => OutputDestination::File(p),
            None => OutputDestination::Stdout,
        }
    }
}

/// Resolves a path, converting relative paths to absolute by joining with cwd.
pub(crate) fn resolve_output_path(path: impl AsRef<std::path::Path>) -> PathBuf {
    let path = path.as_ref();
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    }
}

/// Formats a bytes-per-second rate without the `/s` unit (e.g. `12.4 MB`);
/// the JSON `bytes_per_sec` fields carry this form, so their name states the
/// unit. Sub-KB rates keep one decimal place so slow but nonzero traffic
/// doesn't round down to `0 B`.
pub(crate) fn format_bytes_per_sec(rate: f64) -> String {
    if rate < 1024.0 {
        format!("{rate:.1} B")
    } else {
        format_bytes(rate.round() as u64)
    }
}

/// Formats an optional bytes-per-second rate for display (e.g. `12.4 MB/s`),
/// or `-` when absent.
pub(crate) fn format_throughput(rate: Option<f64>) -> String {
    rate.map_or_else(
        || "-".to_string(),
        |v| format!("{}/s", format_bytes_per_sec(v)),
    )
}

static USE_COLORS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

pub(crate) fn set_use_colors(value: bool) {
    let _ = USE_COLORS.set(value);
}

pub(crate) fn use_colors() -> bool {
    *USE_COLORS.get().unwrap_or(&false)
}

#[cfg(feature = "hotpath-cpu-meta")]
pub(crate) fn cyan(text: &str) -> String {
    if use_colors() {
        format!("\x1b[1;36m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

struct TruncatingWriter {
    buf: String,
    limit: usize,
    truncated: bool,
}

impl std::fmt::Write for TruncatingWriter {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        if self.truncated {
            return Ok(());
        }

        let remaining = self.limit.saturating_sub(self.buf.len());
        if remaining == 0 {
            if !s.is_empty() {
                self.truncated = true;
            }
            return Ok(());
        }

        let end = floor_char_boundary(s, s.len().min(remaining));

        if end < s.len() {
            self.truncated = true;
        }

        self.buf.push_str(&s[..end]);
        Ok(())
    }
}

pub fn format_debug_truncated(value: &impl std::fmt::Debug) -> String {
    let _suspend = crate::lib_on::SuspendAllocTracking::new();
    use std::fmt::Write;
    let limit = MAX_LOG_LEN.saturating_sub(3);
    let mut writer = TruncatingWriter {
        buf: String::with_capacity(64),
        limit,
        truncated: false,
    };
    let _ = write!(writer, "{:?}", value);

    if writer.truncated {
        writer.buf.push_str("...");
    }

    writer.buf
}

#[cfg(test)]
mod tests {
    use crate::output_on::*;

    #[test]
    fn test_format_debug_truncated() {
        let truncate_point = MAX_LOG_LEN.saturating_sub(3);

        let test_cases: Vec<(&str, String)> = vec![
            (
                "japanese at boundary",
                format!("{}リプライ", "a".repeat(truncate_point - 2)),
            ),
            ("emoji", "🦀".repeat(500)),
            ("chinese", "拥抱中文字符测试".repeat(200)),
            (
                "2-byte at boundary",
                format!("{}ñoño", "a".repeat(truncate_point - 1)),
            ),
        ];

        for (name, input) in test_cases {
            let result = format_debug_truncated(&input);
            assert!(
                result.chars().count() > 0,
                "{}: result should have chars",
                name
            );
            if input.len() > *MAX_LOG_LEN {
                assert!(
                    result.ends_with("..."),
                    "{}: truncated result should end with '...'",
                    name
                );
            }
        }
    }

    #[test]
    fn test_format_bytes_per_sec() {
        assert_eq!(format_bytes_per_sec(0.33), "0.3 B");
        assert_eq!(format_bytes_per_sec(1023.9), "1023.9 B");
        assert_eq!(format_bytes_per_sec(1536.0), "1.5 KB");
        assert_eq!(format_bytes_per_sec(26_004_684.8), "24.8 MB");
    }

    #[test]
    fn test_format_throughput() {
        assert_eq!(format_throughput(None), "-");
        assert_eq!(format_throughput(Some(0.33)), "0.3 B/s");
        assert_eq!(format_throughput(Some(512.0)), "512.0 B/s");
        assert_eq!(format_throughput(Some(1023.9)), "1023.9 B/s");
        assert_eq!(format_throughput(Some(1536.0)), "1.5 KB/s");
        assert_eq!(format_throughput(Some(26_004_684.8)), "24.8 MB/s");
    }
}

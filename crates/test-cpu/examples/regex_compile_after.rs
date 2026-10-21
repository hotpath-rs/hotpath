//! Fix for the per-line `Regex` compilation shown in the
//! `regex_compile_before` example: the pattern is compiled once into a
//! `LazyLock` static. Compilation disappears from the cpu report and matching
//! becomes too cheap to register - the next pipeline hotspot, `count_levels`,
//! now tops the report instead.
//!
//! Run with:
//!   cargo run --release -p test-cpu --example regex_compile_after --features hotpath-cpu

use std::sync::LazyLock;

use regex::Regex;

const LINES: usize = 10_000;

// The pattern is compiled once, on first use.
static LOG_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\d{4}-\d{2}-\d{2} (ERROR|WARN|INFO) .+").unwrap());

#[hotpath::measure]
fn is_valid(line: &str) -> bool {
    LOG_LINE.is_match(line)
}

#[hotpath::measure]
fn parse_logs(lines: &[String]) -> usize {
    lines.iter().filter(|line| is_valid(line)).count()
}

// The other pipeline stage: aggregate log level counts over the parsed lines.
#[hotpath::measure]
fn count_levels(lines: &[String]) -> [usize; 3] {
    let mut counts = [0usize; 3];
    for line in lines {
        for (i, level) in ["ERROR", "WARN", "INFO"].iter().enumerate() {
            if line.contains(level) {
                counts[i] += 1;
            }
        }
    }
    counts
}

fn generate_lines() -> Vec<String> {
    let levels = ["ERROR", "WARN", "INFO", "TRACE"];
    (0..LINES)
        .map(|i| {
            let level = levels[i % levels.len()];
            format!("2026-08-{:02} {level} request {i} finished", i % 28 + 1)
        })
        .collect()
}

#[hotpath::main(report = "functions-timing,functions-cpu")]
fn main() {
    let lines = generate_lines();
    let valid = parse_logs(&lines);
    let mut counts = [0usize; 3];
    for _ in 0..300 {
        counts = count_levels(&lines);
    }
    println!("{valid}/{LINES} lines valid, level counts: {counts:?}");
}

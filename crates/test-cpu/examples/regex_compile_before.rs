//! Demonstrates finding a CPU hotspot with sampling: a fresh `Regex` is
//! compiled for every parsed line. Compilation is measured separately from
//! matching, so the cpu report attributes over 90% of samples to
//! `compile_pattern` - pinpointing pattern compilation, not matching, as the
//! hotspot. Compare with the `regex_compile_after` example.
//!
//! Run with:
//!   cargo run --release -p test-cpu --example regex_compile_before --features hotpath-cpu

use regex::Regex;

const LINES: usize = 10_000;

#[hotpath::measure]
fn compile_pattern() -> Regex {
    Regex::new(r"^\d{4}-\d{2}-\d{2} (ERROR|WARN|INFO) .+").unwrap()
}

// A fresh Regex is compiled for every line: parsing the pattern costs far
// more CPU than matching it.
#[hotpath::measure]
fn is_valid(line: &str) -> bool {
    compile_pattern().is_match(line)
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

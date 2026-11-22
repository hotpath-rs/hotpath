//! Helpers shared by the integration tests. Each test file pulls this in with
//! `#[path = "common/support.rs"] mod common;` and uses only a subset.
#![allow(dead_code)]

pub(crate) mod callgrind;
pub(crate) mod example;
pub(crate) mod prometheus;
pub(crate) mod service;

#[cfg(feature = "json")]
pub(crate) mod endpoints;
#[cfg(feature = "json")]
pub(crate) mod report;

pub(crate) fn path_sep() -> &'static str {
    if cfg!(windows) {
        "\\"
    } else {
        "/"
    }
}

/// The first JSON value in `stdout`; trailing log lines follow the report.
/// Needs no `json` feature, unlike `report::parse_report`.
#[track_caller]
pub(crate) fn first_json_value(stdout: &str) -> serde_json::Value {
    let json_start = stdout.find('{').expect("No JSON report in output");
    serde_json::Deserializer::from_str(&stdout[json_start..])
        .into_iter::<serde_json::Value>()
        .next()
        .expect("No JSON value in output")
        .expect("Failed to parse JSON report")
}

#[track_caller]
pub(crate) fn assert_contains_all(haystack: &str, all_expected: &[&str]) {
    for expected in all_expected {
        assert!(
            haystack.contains(expected),
            "Expected:\n{expected}\n\nGot:\n{haystack}",
        );
    }
}

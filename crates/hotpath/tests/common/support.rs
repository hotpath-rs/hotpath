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

#[track_caller]
pub(crate) fn assert_contains_all(haystack: &str, all_expected: &[&str]) {
    for expected in all_expected {
        assert!(
            haystack.contains(expected),
            "Expected:\n{expected}\n\nGot:\n{haystack}",
        );
    }
}

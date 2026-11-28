//! Shared code of the bare-metal variability benchmarks: the One Billion Row
//! Challenge solver and its deterministic dataset generator. The benchmark
//! binaries are the examples; `.github/workflows/benchmark-metal.yml` runs them.

pub mod generate;
pub mod onebrc;
mod stations;

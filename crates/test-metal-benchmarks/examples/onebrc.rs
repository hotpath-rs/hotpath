//! Multi-threaded One Billion Row Challenge benchmark on a generated dataset
//! (see `generate_1brc`). The file is read into memory before anything is
//! measured, then `MEASURED_RUNS` measured `run` calls follow (split,
//! aggregate on 4 threads, merge, sort, format), with no warm-up.
//! Every output is compared with the expected output file. With `-` instead
//! of the expected file it prints the output and exits, which is how
//! `data/expected-100M.txt` was produced.
//!
//! Run with:
//!   cargo run --release -p test-metal-benchmarks --example onebrc --features hotpath -- <measurements.txt> <expected.txt>

use test_metal_benchmarks::onebrc::run;

const MEASURED_RUNS: usize = 5;

#[hotpath::main(percentiles = [50, 95])]
fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(data_path), Some(expected_path)) = (args.next(), args.next()) else {
        panic!("usage: onebrc <measurements.txt> <expected.txt>");
    };
    let data = std::fs::read(&data_path).expect("read measurements");
    if expected_path == "-" {
        print!("{}", run(&data));
        return;
    }
    let expected = std::fs::read_to_string(&expected_path).expect("read expected output");

    for _ in 0..MEASURED_RUNS {
        assert_eq!(run(&data), expected, "measured output");
    }
}

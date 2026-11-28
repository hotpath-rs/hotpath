//! The 1BRC solver against a naive single-threaded reference, on edge cases
//! and on a small generated dataset, with 1 and 4 threads.

use std::collections::BTreeMap;

use test_metal_benchmarks::generate::{generate, SEED};
use test_metal_benchmarks::onebrc::solve;

/// Independent reference: `str` parsing through `f64`, a `BTreeMap<String, _>`
/// and rounding of the mean half towards positive infinity.
fn reference(input: &str) -> String {
    let mut stats: BTreeMap<&str, (i64, i64, i64, i64)> = BTreeMap::new();
    for line in input.lines() {
        let (name, temp) = line.rsplit_once(';').unwrap();
        let t = (temp.parse::<f64>().unwrap() * 10.0).round() as i64;
        let e = stats.entry(name).or_insert((t, t, 0, 0));
        e.0 = e.0.min(t);
        e.1 = e.1.max(t);
        e.2 += t;
        e.3 += 1;
    }
    let fmt = |tenths: i64| format!("{:.1}", tenths as f64 / 10.0).replace("-0.0", "0.0");
    let body: Vec<String> = stats
        .iter()
        .map(|(name, &(min, max, sum, count))| {
            let mean = (sum as f64 / count as f64 + 0.5).floor() as i64;
            format!("{name}={}/{}/{}", fmt(min), fmt(mean), fmt(max))
        })
        .collect();
    format!("{{{}}}\n", body.join(", "))
}

fn check(input: &str) {
    let expected = reference(input);
    for threads in [1, 2, 3, 4, 7] {
        assert_eq!(
            solve(input.as_bytes(), threads),
            expected,
            "{threads} threads"
        );
    }
}

#[test]
fn edge_cases() {
    check(
        "Abha;-23.0\n\
         Abha;59.2\n\
         Abha;18.0\n\
         Zürich;-0.1\n\
         Zürich;0.0\n\
         Washington, D.C.;99.9\n\
         Washington, D.C.;-99.9\n\
         Tie;0.1\n\
         Tie;0.2\n\
         NegTie;-0.1\n\
         NegTie;-0.2\n\
         Single;-5.5\n\
         İzmir;12.3\n",
    );
}

#[test]
fn one_row_and_more_threads_than_rows() {
    check("Solo;1.0\n");
    check("A;1.0\nB;2.0\n");
}

#[test]
fn generated_dataset() {
    let mut data = Vec::new();
    generate(50_000, SEED, &mut data).unwrap();
    check(std::str::from_utf8(&data).unwrap());
}

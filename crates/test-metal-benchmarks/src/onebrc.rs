//! A straightforward multi-threaded One Billion Row Challenge solver: the
//! input is split into one chunk per thread at line boundaries, each thread
//! aggregates its chunk into a hash map, and the maps are merged and printed
//! in the challenge's format, `{Abha=-23.0/18.0/59.2, ...}`.
//!
//! The maps use `DefaultHasher` with its fixed keys instead of `RandomState`,
//! so hashing, probing and iteration order are identical in every run.

use std::collections::{BTreeMap, HashMap};
use std::hash::{BuildHasherDefault, DefaultHasher};
use std::thread;

use crate::generate::write_tenths;

/// Worker threads of every benchmark run, on every runner.
pub const THREADS: usize = 4;

#[derive(Clone, Copy)]
pub(crate) struct Stats {
    min: i16,
    max: i16,
    sum: i64,
    count: u64,
}

type Map<'a> = HashMap<&'a [u8], Stats, BuildHasherDefault<DefaultHasher>>;

/// The measured benchmark unit: splitting, aggregating on `THREADS` threads,
/// merging, sorting and formatting. Excludes reading the input.
#[hotpath::measure]
pub fn run(data: &[u8]) -> String {
    solve(data, THREADS, true)
}

/// Solves `data` on `threads` threads. With `measured` false no measured
/// function is called, so warm-up runs add nothing to the report.
pub fn solve(data: &[u8], threads: usize, measured: bool) -> String {
    let chunks = split(data, threads);
    let maps: Vec<Map> = thread::scope(|s| {
        let handles: Vec<_> = chunks
            .iter()
            .enumerate()
            .map(|(i, chunk)| {
                thread::Builder::new()
                    .name(format!("1brc-worker-{i}"))
                    .spawn_scoped(s, move || {
                        if measured {
                            process_chunk_measured(chunk)
                        } else {
                            process_chunk(chunk)
                        }
                    })
                    .expect("spawn 1brc worker")
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("1brc worker panicked"))
            .collect()
    });
    let merged = if measured {
        merge_measured(maps)
    } else {
        merge(maps)
    };
    format(&merged)
}

/// Splits `data` into at most `parts` chunks, each ending at a newline.
fn split(data: &[u8], parts: usize) -> Vec<&[u8]> {
    let mut chunks = Vec::with_capacity(parts);
    let mut start = 0;
    for i in 1..=parts {
        if start >= data.len() {
            break;
        }
        let mut end = if i == parts {
            data.len()
        } else {
            (data.len() * i / parts).max(start + 1)
        };
        while end < data.len() && data[end - 1] != b'\n' {
            end += 1;
        }
        chunks.push(&data[start..end]);
        start = end;
    }
    chunks
}

#[hotpath::measure]
fn process_chunk_measured(chunk: &[u8]) -> Map<'_> {
    process_chunk(chunk)
}

fn process_chunk(chunk: &[u8]) -> Map<'_> {
    let mut map = Map::default();
    for line in chunk.split(|&b| b == b'\n') {
        if line.is_empty() {
            continue;
        }
        let sep = line
            .iter()
            .rposition(|&b| b == b';')
            .expect("row without ';'");
        let temp = parse_tenths(&line[sep + 1..]);
        map.entry(&line[..sep])
            .and_modify(|s| {
                s.min = s.min.min(temp);
                s.max = s.max.max(temp);
                s.sum += i64::from(temp);
                s.count += 1;
            })
            .or_insert(Stats {
                min: temp,
                max: temp,
                sum: i64::from(temp),
                count: 1,
            });
    }
    map
}

/// Parses `-?\d{1,2}\.\d` into tenths.
fn parse_tenths(s: &[u8]) -> i16 {
    let (negative, digits) = match s.first() {
        Some(b'-') => (true, &s[1..]),
        _ => (false, s),
    };
    let mut value: i16 = 0;
    for &b in digits {
        if b != b'.' {
            assert!(b.is_ascii_digit(), "bad temperature {s:?}");
            value = value * 10 + i16::from(b - b'0');
        }
    }
    if negative {
        -value
    } else {
        value
    }
}

#[hotpath::measure]
fn merge_measured(maps: Vec<Map<'_>>) -> BTreeMap<&[u8], Stats> {
    merge(maps)
}

fn merge(maps: Vec<Map<'_>>) -> BTreeMap<&[u8], Stats> {
    let mut merged: BTreeMap<&[u8], Stats> = BTreeMap::new();
    for map in maps {
        for (name, s) in map {
            merged
                .entry(name)
                .and_modify(|m| {
                    m.min = m.min.min(s.min);
                    m.max = m.max.max(s.max);
                    m.sum += s.sum;
                    m.count += s.count;
                })
                .or_insert(s);
        }
    }
    merged
}

fn format(merged: &BTreeMap<&[u8], Stats>) -> String {
    let mut out = Vec::with_capacity(merged.len() * 32);
    out.push(b'{');
    for (i, (name, s)) in merged.iter().enumerate() {
        if i > 0 {
            out.extend_from_slice(b", ");
        }
        out.extend_from_slice(name);
        out.push(b'=');
        write_tenths(&mut out, i32::from(s.min));
        out.push(b'/');
        write_tenths(&mut out, mean_tenths(s.sum, s.count));
        out.push(b'/');
        write_tenths(&mut out, i32::from(s.max));
    }
    out.extend_from_slice(b"}\n");
    String::from_utf8(out).expect("station names are UTF-8")
}

/// The mean of `sum / count` tenths, rounded half towards positive infinity.
fn mean_tenths(sum: i64, count: u64) -> i32 {
    let count = count as i64;
    (2 * sum + count).div_euclid(2 * count) as i32
}

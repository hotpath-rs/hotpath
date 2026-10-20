//! Parser for Callgrind's profile format
//! (https://valgrind.org/docs/manual/cl-format.html), reduced to what marker
//! attribution needs: the `Ir` total and the inclusive `Ir` of every marker
//! function.
//!
//! A marker is a nested `fn __hotpath_cg_<16 hex>` the measure macros emit
//! around each measured body. Its inclusive cost is the sum of the call-edge
//! costs (the cost line after `calls=`) into it, i.e. the sum over calls of
//! each call's inclusive cost - the same semantics as timing totals, so a
//! recursive function counts nested activations once per enclosing level.

use std::collections::HashMap;

pub(crate) const MARKER_PREFIX: &str = "__hotpath_cg_";
const MARKER_HASH_LEN: usize = 16;

#[derive(Debug)]
pub(crate) struct Dump {
    /// `creator:` header, e.g. `callgrind-3.24.0`.
    pub(crate) creator: Option<String>,
    /// Text after `desc: Trigger:`, e.g. `Client Request: <tag>`.
    pub(crate) trigger: Option<String>,
    pub(crate) total_ir: u64,
    /// Inclusive `Ir` per marker ident, summed over every instantiation.
    pub(crate) inclusive: HashMap<String, u64>,
}

#[derive(Default)]
struct Names {
    /// Compressed `(id)` -> marker ident of that name, if it is a marker.
    markers: HashMap<u64, Option<String>>,
}

impl Names {
    /// Resolves an `fn=`/`cfn=` value: `(id) name` defines `id`, `(id)`
    /// refers back to it, a bare `name` is uncompressed.
    fn resolve(&mut self, value: &str, line_no: usize) -> Result<Option<String>, String> {
        let value = value.trim();
        let Some(rest) = value.strip_prefix('(') else {
            return Ok(marker_of(value).map(str::to_string));
        };
        let (id, name) = rest
            .split_once(')')
            .ok_or_else(|| format!("line {line_no}: unterminated name id in {value:?}"))?;
        let id: u64 = id
            .trim()
            .parse()
            .map_err(|_| format!("line {line_no}: invalid name id in {value:?}"))?;
        let name = name.trim();
        if name.is_empty() {
            // A reference to an id this file never defined cannot be a marker
            // we know about; Callgrind always defines before referring.
            Ok(self.markers.get(&id).cloned().flatten())
        } else {
            let marker = marker_of(name).map(str::to_string);
            self.markers.insert(id, marker.clone());
            Ok(marker)
        }
    }
}

/// The marker ident a symbol names when it is the marker function itself:
/// the ident must sit outside every `<...>`, so callers that only carry the
/// marker in a generic argument (the future wrapper's `poll`,
/// `drop_in_place::<...>`) do not match.
pub(crate) fn marker_of(name: &str) -> Option<&str> {
    let bytes = name.as_bytes();
    let mut depth: usize = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'<' => depth += 1,
            b'>' if i > 0 && bytes[i - 1] == b'-' => {}
            b'>' => depth = depth.saturating_sub(1),
            b'_' if depth == 0 && name[i..].starts_with(MARKER_PREFIX) => {
                let hash_start = i + MARKER_PREFIX.len();
                let end = hash_start + MARKER_HASH_LEN;
                let is_hash = bytes
                    .get(hash_start..end)
                    .is_some_and(|h| h.iter().all(|b| b.is_ascii_hexdigit()));
                let ends_ident = bytes
                    .get(end)
                    .is_none_or(|b| !(b.is_ascii_alphanumeric() || *b == b'_'));
                if is_hash && ends_ident {
                    return Some(&name[i..end]);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure)]
pub(crate) fn parse(input: &str) -> Result<Dump, String> {
    let mut names = Names::default();
    let mut ir_index: Option<usize> = None;
    let mut position_columns: usize = 1;
    let mut summary: Option<u64> = None;
    let mut totals: Option<u64> = None;
    let mut trigger: Option<String> = None;
    let mut creator: Option<String> = None;
    let mut inclusive: HashMap<String, u64> = HashMap::new();

    let mut callee: Option<String> = None;
    // Set by `calls=`: the next cost line is that call's inclusive cost.
    let mut call_target: Option<Option<String>> = None;

    for (idx, raw) in input.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if let Some(target) = call_target.take() {
            let ir_index =
                ir_index.ok_or_else(|| format!("line {line_no}: cost before events:"))?;
            let cost = cost_at(line, position_columns, ir_index, line_no)?;
            if let Some(target) = target {
                *inclusive.entry(target).or_insert(0) += cost;
            }
            continue;
        }

        if !line.as_bytes()[0].is_ascii_alphabetic() {
            // A self-cost line; marker attribution only needs call edges.
            continue;
        }

        if let Some(value) = line.strip_prefix("fn=") {
            // Defines a compressed name the later `cfn=` lines may refer to.
            names.resolve(value, line_no)?;
        } else if let Some(value) = line.strip_prefix("cfn=") {
            callee = names.resolve(value, line_no)?;
        } else if line.starts_with("calls=") {
            call_target = Some(callee.take());
        } else if let Some(value) = line.strip_prefix("events:") {
            let index = value.split_whitespace().position(|e| e == "Ir");
            ir_index = Some(index.ok_or_else(|| format!("line {line_no}: no Ir event"))?);
        } else if let Some(value) = line.strip_prefix("positions:") {
            position_columns = value.split_whitespace().count().max(1);
        } else if let Some(value) = line.strip_prefix("summary:") {
            summary = Some(header_cost(value, ir_index, line_no)?);
        } else if let Some(value) = line.strip_prefix("totals:") {
            totals = Some(header_cost(value, ir_index, line_no)?);
        } else if let Some(value) = line.strip_prefix("desc: Trigger:") {
            trigger = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("creator:") {
            creator = Some(value.trim().to_string());
        }
    }

    if call_target.is_some() {
        return Err("truncated dump: calls= without a cost line".to_string());
    }
    let total_ir = totals
        .or(summary)
        .ok_or_else(|| "no summary: or totals: line".to_string())?;

    Ok(Dump {
        creator,
        trigger,
        total_ir,
        inclusive,
    })
}

fn header_cost(value: &str, ir_index: Option<usize>, line_no: usize) -> Result<u64, String> {
    let ir_index = ir_index.ok_or_else(|| format!("line {line_no}: totals before events:"))?;
    parse_count(value.split_whitespace().nth(ir_index), line_no)
}

/// `Ir` of a cost line: position columns first (`0`, `+3`, `-1`, `*`,
/// `0x...`), then one value per event; trailing zero events may be omitted.
fn cost_at(
    line: &str,
    position_columns: usize,
    ir_index: usize,
    line_no: usize,
) -> Result<u64, String> {
    let mut fields = line.split_whitespace().skip(position_columns);
    match fields.nth(ir_index) {
        Some(value) => parse_count(Some(value), line_no),
        None => Ok(0),
    }
}

fn parse_count(value: Option<&str>, line_no: usize) -> Result<u64, String> {
    let value = value.unwrap_or("0");
    value
        .parse()
        .map_err(|_| format!("line {line_no}: invalid cost {value:?}"))
}

#[cfg(test)]
mod tests {
    use crate::lib_on::functions::callgrind::parser::{marker_of, parse};

    #[test]
    fn marker_of_matches_only_the_marker_function() {
        let m = "__hotpath_cg_00112233aabbccdd";
        let cases = [
            (
                format!("app::work::{m}::<u64, app::work::{{closure#0}}>"),
                true,
            ),
            (
                format!("<app::Worker as app::Job>::run::{m}::<u64, _>"),
                true,
            ),
            (format!("<app::Worker>::inherent::{m}"), true),
            (format!("app::rec::{m}::<u64, _>'2"), true),
            (
                format!("<hotpath::CallgrindFuture<F, app::f::{m}<F>> as Future>::poll"),
                false,
            ),
            (
                format!("core::ptr::drop_in_place::<app::f::{m}<fn() -> u64>>"),
                false,
            ),
            (format!("app::f::{m}0"), false),
            ("app::f::__hotpath_cg_0011".to_string(), false),
        ];
        for (name, expected) in cases {
            assert_eq!(marker_of(&name) == Some(m), expected, "{name}");
        }
    }

    // Trimmed from a real Valgrind 3.24 dump of test-callgrind's
    // `instructions` example: every edge into a marker kept, plus edges into
    // symbols that carry a marker only in generic arguments (the
    // `InstrumentedFuture` poll and constructor, a drop glue).
    #[test]
    fn real_dump_attributes_inclusive_cost_per_marker() {
        let dump = parse(include_str!("fixtures/instructions.callgrind")).unwrap();

        assert_eq!(dump.creator.as_deref(), Some("callgrind-3.24.0"));
        assert_eq!(dump.trigger.as_deref(), Some("Client Request: hotpath-8-1"));
        assert_eq!(dump.total_ir, 40_101_943);

        let mut inclusive: Vec<(&str, u64)> = dump
            .inclusive
            .iter()
            .map(|(marker, ir)| (marker.as_str(), *ir))
            .collect();
        inclusive.sort();
        assert_eq!(
            inclusive,
            [
                ("__hotpath_cg_0a43f54bf9e4444b", 1_800_012), // <Worker as Job>::run
                ("__hotpath_cg_189c9d1f4fb56614", 3_600_024), // free_fn, 2 calls
                ("__hotpath_cg_3c585534bb08202f", 1_800_021), // Worker::first_mut
                ("__hotpath_cg_77d1f822c0830eed", 1_800_012), // logged_fn
                ("__hotpath_cg_8c1f1ae4ff049387", 1_800_012), // Worker::inherent
                ("__hotpath_cg_9624c3670a93e126", 1_800_021), // tracked_future
                ("__hotpath_cg_ab2649e9e6013710", 1_800_012), // labeled_fn
                // recursive(3): 7_201_112 from main plus 10_801_146 from its
                // `'2` recursion copy - each level's inclusive cost.
                ("__hotpath_cg_c5edf7c7a9c739da", 18_002_258),
                // async_fn: 10 polls over two calls, one of them spawned.
                ("__hotpath_cg_ecb39ecd98d0fc28", 14_401_595),
                // generic_fn::<u32> and ::<u64> summed.
                ("__hotpath_cg_fdfe56723f88fc60", 3_600_024),
            ]
        );
    }

    #[test]
    fn format_variants() {
        let m = "__hotpath_cg_00112233aabbccdd";
        let input = format!(
            "\
# callgrind format
version: 1
creator: callgrind-3.99.0
positions: instr line
events: Dr Ir Dw
summary: 5 1000 7

ob=(1) /bin/app
fl=(1) src/main.rs
fn=(1) app::main
0x10 3 1 2 3
+4 +1 9
cfn={m}::<u64>
calls=2 0x20 10
* -2 4 600 9
cfn=(2) app::other
calls=1 +8 *
+2 * 1 50
cfn=(3) app::f::{m}::<u32>
calls=1 0x30 12
+1 +1 0 300
fn=(4) app::other
cfn=(3)
calls=1 0x30 12
0x40 7 0 1
"
        );
        let dump = parse(&input).unwrap();

        // `totals:` is absent, so `summary:` is the total.
        assert_eq!(dump.total_ir, 1000);
        assert_eq!(dump.trigger, None);
        // 600 uncompressed + 300 via `(3)` + 1 through the `(3)` back
        // reference, whose cost line omits the last event.
        assert_eq!(dump.inclusive.get(m), Some(&901));
        assert_eq!(dump.inclusive.len(), 1);
    }

    #[test]
    fn rejects_malformed_dumps() {
        let truncated = "events: Ir\nsummary: 1\nfn=(1) a\ncfn=(2) b\ncalls=1 0\n";
        assert!(parse(truncated).unwrap_err().contains("truncated"));

        let no_ir = "events: Dr Dw\nsummary: 1 1\n";
        assert!(parse(no_ir).unwrap_err().contains("no Ir event"));

        let no_total = "events: Ir\nfn=(1) a\n0 5\n";
        assert!(parse(no_total).unwrap_err().contains("summary"));

        let bad_cost = "events: Ir\nsummary: 1\nfn=(1) a\ncfn=(2) b\ncalls=1 0\n0 x\n";
        assert!(parse(bad_cost).unwrap_err().contains("line 6"));
    }
}

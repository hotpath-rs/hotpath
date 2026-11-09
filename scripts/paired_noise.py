#!/usr/bin/env python3
"""Noise of same-runner and historical comparisons, from one run of
`.github/workflows/cloud-paired-noise.yml`.

Every job measured one commit twice, as baseline and head, so every
difference is noise:

- same runner: job k's head against job k's baseline (what a pair compares),
- historical: job k's head against job j's baseline, j != k (another runner,
  what a comparison against the base branch's push report does).

Run with:

    gh run download <run id> -D noise
    python3 scripts/paired_noise.py noise

Prints, per benchmark and metric, the distribution of the percentage
differences (median, P5-P95), the share above each regression threshold
(a false regression: the code did not change) and the spread of every
measurement across all runs. Standard library only.
"""

import json
import re
import statistics
import sys
from collections import defaultdict
from pathlib import Path

THRESHOLDS = [1.0, 2.0, 5.0, 10.0, 20.0]
# A value under this many ns or bytes is too small for a ratio to mean much.
MIN_VALUE = 1.0

DURATION = {"ns": 1, "µs": 1e3, "us": 1e3, "ms": 1e6, "s": 1e9}
BYTES = {"B": 1, "KB": 1e3, "MB": 1e6, "GB": 1e9, "KiB": 1024, "MiB": 1024**2, "GiB": 1024**3}


def parse_value(raw):
    """A report cell as a number: `18.6 ms` in ns, `1216768 B` in bytes, a
    count as is. None for anything else (a missing or non-numeric cell)."""
    if isinstance(raw, (int, float)):
        return float(raw)
    if not isinstance(raw, str):
        return None
    m = re.fullmatch(r"\s*([0-9][0-9,]*(?:\.[0-9]+)?)\s*([A-Za-zµ]*)\s*", raw)
    if not m:
        return None
    number = float(m.group(1).replace(",", ""))
    unit = m.group(2)
    if unit == "":
        return number
    for table in (DURATION, BYTES):
        if unit in table:
            return number * table[unit]
    return None


# (section, column): the metrics a policy judges.
METRICS = [
    ("functions_timing", "avg"),
    ("functions_timing", "total"),
    ("functions_alloc", "total"),
    ("functions_icount", "total"),
]


def measurements(report):
    """{(section, column, function): value} of one report."""
    out = {}
    for section, column in METRICS:
        for entry in (report.get(section) or {}).get("data", []):
            value = parse_value(entry.get(column))
            if value is not None:
                out[(section, column, entry["name"])] = value
    return out


def load(root):
    """{benchmark: {job: {"base": report, "head": report}}} from the
    downloaded artifacts `hotpath-baseline-<attempt>-<sha>-<benchmark>-<job>`
    and `hotpath-report-<benchmark>-<job>`."""
    runs = defaultdict(lambda: defaultdict(dict))
    for path in sorted(Path(root).glob("*/report.json")):
        name = path.parent.name
        m = re.fullmatch(r"hotpath-baseline-\d+-[0-9a-f]{40}-(.+)-(\d+)", name)
        side = "base"
        if not m:
            m = re.fullmatch(r"hotpath-report-(.+)-(\d+)", name)
            side = "head"
        if not m:
            continue
        runs[m.group(1)][int(m.group(2))][side] = json.loads(path.read_text())
    return runs


def percent(base, head):
    if base < MIN_VALUE:
        return None
    return (head - base) / base * 100.0


def quantile(values, q):
    values = sorted(values)
    if not values:
        return float("nan")
    i = (len(values) - 1) * q
    lo, hi = int(i), min(int(i) + 1, len(values) - 1)
    return values[lo] + (values[hi] - values[lo]) * (i - lo)


def summarize(diffs):
    if not diffs:
        return None
    row = {
        "n": len(diffs),
        "median": statistics.median(diffs),
        "p5": quantile(diffs, 0.05),
        "p95": quantile(diffs, 0.95),
    }
    for t in THRESHOLDS:
        # A regression is an increase beyond the bar, as a policy judges it.
        row[t] = sum(d > t for d in diffs) / len(diffs) * 100.0
    return row


def main(root):
    runs = load(root)
    if not runs:
        sys.exit(f"no paired-noise artifacts under {root}")
    for benchmark, jobs in sorted(runs.items()):
        complete = {k: v for k, v in jobs.items() if "base" in v and "head" in v}
        print(f"## {benchmark}: {len(complete)} complete jobs of {len(jobs)}\n")
        cpus = defaultdict(int)
        for sides in complete.values():
            cpus[(sides["base"].get("user_metadata") or {}).get("cpu", "?")] += 1
        print("CPU models: " + ", ".join(f"{c} ({n})" for c, n in sorted(cpus.items())) + "\n")
        values = {k: {s: measurements(r) for s, r in v.items()} for k, v in complete.items()}

        header = "| metric | comparison | n | median | P5 | P95 | " + " | ".join(
            f"> {t:g}%" for t in THRESHOLDS
        ) + " |"
        print(header)
        print("|" + " --- |" * (header.count("|") - 1))
        for section, column in METRICS:
            same, historical, spread = [], [], []
            keys = set()
            for v in values.values():
                keys |= {k for k in v["head"] if k[:2] == (section, column)}
            for key in sorted(keys):
                for k, v in values.items():
                    head = v["head"].get(key)
                    if head is None:
                        continue
                    base = v["base"].get(key)
                    if base is not None and (d := percent(base, head)) is not None:
                        same.append(d)
                    for j, other in values.items():
                        base = other["base"].get(key)
                        if j != k and base is not None and (d := percent(base, head)) is not None:
                            historical.append(d)
                # Spread of one function's value over every run, both sides.
                all_runs = [v[s][key] for v in values.values() for s in ("base", "head") if key in v[s]]
                if len(all_runs) > 1 and statistics.mean(all_runs) >= MIN_VALUE:
                    spread.append(statistics.stdev(all_runs) / statistics.mean(all_runs) * 100.0)
            for label, diffs in (("same runner", same), ("historical", historical)):
                row = summarize(diffs)
                if row is None:
                    continue
                cells = " | ".join(f"{row[t]:.1f}%" for t in THRESHOLDS)
                print(
                    f"| {section}.{column} | {label} | {row['n']} | {row['median']:+.2f}% | "
                    f"{row['p5']:+.2f}% | {row['p95']:+.2f}% | {cells} |"
                )
            if spread:
                print(
                    f"| {section}.{column} | spread (CV over all runs) | {len(spread)} fns | "
                    f"{statistics.median(spread):.3f}% | {min(spread):.3f}% | {max(spread):.3f}% |"
                    + " |" * len(THRESHOLDS)
                )
        print()


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    main(sys.argv[1])

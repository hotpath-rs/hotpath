"""Extracts inclusive Ir of the measured functions from one
`callgrind_annotate --inclusive=yes` output as a JSON line.

Usage: extract.py <annotate.txt> <program-stdout.txt> <job> <run>
"""

import json
import re
import sys

FUNCTIONS = [
    "sync_checksum",
    "sync_hashmap_random",
    "sync_hashmap_fixed",
    "sync_alloc",
    "sync_memchr",
    "async_yield_compute",
    "async_sleep_compute",
    "async_channel_pipeline",
]


def inclusive_ir(lines, name):
    sync_symbol = f"???:callgrind_poc::{name} "
    # Async bodies are attributed to the never-inlined poll wrapper.
    async_prefix = f"???:<callgrind_poc::NoInline<callgrind_poc::{name}::"
    for line in lines:
        m = re.match(r"\s*([\d,]+) \(.*?\)\s+(.*)$", line)
        if not m or "'" in m.group(2) or "drop_glue" in m.group(2):
            continue
        symbol = m.group(2)
        if symbol.startswith(sync_symbol) or symbol.startswith(async_prefix):
            return int(m.group(1).replace(",", ""))
    return None


def main():
    annotate_path, stdout_path, job, run = sys.argv[1:5]
    lines = open(annotate_path).read().splitlines()
    cpu = "unknown"
    for line in open("/proc/cpuinfo"):
        if line.startswith("model name"):
            cpu = line.split(":", 1)[1].strip()
            break
    features = next(
        (l.split(" ", 1)[1].strip() for l in open(stdout_path) if l.startswith("cpu-features")),
        "unknown",
    )
    total = next(
        int(l.split()[0].replace(",", "")) for l in lines if "PROGRAM TOTALS" in l
    )
    record = {
        "job": int(job),
        "run": int(run),
        "cpu": cpu,
        "valgrind_cpu_features": features,
        "counts": {name: inclusive_ir(lines, name) for name in FUNCTIONS},
        "program_total": total,
    }
    print(json.dumps(record))


main()
